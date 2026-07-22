use super::{JitCompiler, JitError, JitSettings, ModuleCompileMetadata};
use crate::atom::Atom;
use crate::jit::RootLocation;
use crate::jit::ir_common::{JIT_DEOPT_SENTINEL, X_REGISTER_COUNT};
use crate::jit::ir_control::{Coverage, coverage};
use crate::jit::ir_exceptions::{
    JIT_STATUS_DEOPT, JIT_STATUS_EXCEPTION, JIT_STATUS_NORMAL, JIT_STATUS_YIELD, JitReturn,
};
use crate::jit::type_info::{FunctionSignature, TypeDescriptor};
use crate::loader::decode::{BifOp, BinaryOp, ComparisonOp, MapOp, Operand, TypeTestOp};
use crate::loader::{Instruction, LambdaEntry};
use crate::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use crate::process::{JitRuntimeContext, Process};
use crate::term::Term;
use crate::term::boxed::{
    Closure, Cons, Float, Map, Tuple, write_closure, write_cons, write_float, write_map,
    write_tuple,
};
use std::collections::HashMap;

type RawJitFn = extern "C" fn(*mut u64, *mut Process) -> JitReturn;

fn call_native(native: &crate::jit::types::NativeCode, registers: &mut [u64]) -> u64 {
    let mut process = Process::new(0, 233);
    call_native_with_process(native, registers, &mut process)
}

fn call_native_with_process(
    native: &crate::jit::types::NativeCode,
    registers: &mut [u64],
    process: &mut Process,
) -> u64 {
    let returned = raw_jit_fn(native)(registers.as_mut_ptr(), process);
    assert_eq!(returned.status, JIT_STATUS_NORMAL);
    returned.value
}

fn call_native_with_process_x_regs(
    native: &crate::jit::types::NativeCode,
    process: &mut Process,
) -> u64 {
    let registers = process.x_regs_mut().as_mut_ptr().cast::<u64>();
    let returned = raw_jit_fn(native)(registers, process);
    assert_eq!(returned.status, JIT_STATUS_NORMAL);
    returned.value
}

fn call_native_status(
    native: &crate::jit::types::NativeCode,
    registers: &mut [u64],
    process: &mut Process,
) -> JitReturn {
    raw_jit_fn(native)(registers.as_mut_ptr(), process)
}

fn raw_jit_fn(native: &crate::jit::types::NativeCode) -> RawJitFn {
    // SAFETY: `NativeCode::call_ptr` is produced by `JitCompiler::compile`
    // with the test ABI `extern "C" fn(*mut u64, *mut Process) -> JitReturn`.
    unsafe { std::mem::transmute(native.call_ptr()) }
}

fn test_lambda(
    function: Atom,
    arity: u8,
    label: u32,
    num_free: u32,
    unique_id: u64,
) -> LambdaEntry {
    LambdaEntry {
        function,
        arity,
        label,
        num_free,
        unique_id,
    }
}

fn heap_closure(
    process: &mut Process,
    module: Atom,
    function_index: u64,
    arity: u8,
    generation: u64,
    unique_id: u64,
    free_vars: &[Term],
) -> Term {
    let words = 7 + free_vars.len();
    let ptr = process.heap_mut().alloc(words).expect("closure heap fits");
    // SAFETY: `ptr` addresses `words` contiguous heap words returned by the
    // process heap allocator for this test allocation.
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, words) };
    write_closure(
        heap,
        module,
        function_index,
        arity,
        generation,
        unique_id,
        free_vars,
    )
    .expect("closure layout fits")
}

fn heap_float(process: &mut Process, value: f64) -> Term {
    let ptr = process.heap_mut().alloc(2).expect("float heap fits");
    // SAFETY: `ptr` addresses two contiguous heap words returned by the
    // process heap allocator for this test allocation.
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, 2) };
    write_float(heap, value).expect("float layout fits")
}

fn heap_map(process: &mut Process, entries: &[(Term, Term)]) -> Term {
    let words = 2 + entries.len() * 2;
    let ptr = process.heap_mut().alloc(words).expect("map heap fits");
    // SAFETY: `ptr` addresses `words` contiguous heap words returned by the
    // process heap allocator for this test allocation.
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, words) };
    let keys = entries.iter().map(|(key, _value)| *key).collect::<Vec<_>>();
    let values = entries
        .iter()
        .map(|(_key, value)| *value)
        .collect::<Vec<_>>();
    write_map(heap, &keys, &values).expect("map layout fits")
}

fn test_module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: HashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

#[test]
fn compiles_return_only_function() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(&[Instruction::Return], Atom::MODULE, Atom::OK, 0)
        .unwrap();

    assert!(!native.call_ptr().is_null());
    assert!(native.stack_maps().is_empty());
}

#[test]
fn compiled_move_writes_register_file() {
    // Re-pinned onto the stack-backed Y substrate (JIT-002 R1): the flat-Y
    // predecessor asserted `Move x1 -> Y(0)` landed at `registers[1024]`, past
    // the X-register file. Under the ruled substrate Y(0) lives in the frame the
    // compiled `allocate` pushes onto the process call stack, so the write is
    // observed through `process.stack().y_reg(0)`, GC-rooted like every Y slot.
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Allocate {
                    stack_need: Operand::Unsigned(1),
                    live: Operand::Unsigned(0),
                },
                Instruction::Move {
                    source: Operand::Integer(42),
                    destination: Operand::X(1),
                },
                Instruction::Move {
                    source: Operand::X(1),
                    destination: Operand::Y(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    process.set_current_module(std::sync::Arc::new(test_module(Atom::MODULE, Vec::new())));
    let returned = call_native_with_process_x_regs(&native, &mut process);

    // X(0) was untouched, so the returned value is the fresh process's NIL.
    assert_eq!(returned, Term::NIL.raw());
    assert_eq!(process.x_reg(1), Term::small_int(42));
    assert_eq!(process.stack().y_reg(0), Ok(Term::small_int(42)));
}

// -- JIT-002 R1: the every-function structural frame set on the stack-backed
// -- Y substrate. Each wall compiles one opcode and observes the process call
// -- stack the collector roots.

/// A process with a live current module so `allocate` can pin a frame, mirroring
/// the interpreter's `push_y_frame`.
fn process_with_current_module() -> Process {
    let mut process = Process::new(0, 233);
    process.set_current_module(std::sync::Arc::new(test_module(Atom::MODULE, Vec::new())));
    process
}

fn compile_body(instructions: &[Instruction]) -> crate::jit::types::NativeCode {
    JitCompiler::new(JitSettings)
        .unwrap()
        .compile(instructions, Atom::MODULE, Atom::OK, 0)
        .unwrap()
}

#[test]
fn compiled_allocate_pushes_frame_with_nil_y_slots() {
    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(process.stack().len(), 1);
    assert_eq!(process.stack().y_reg(0), Ok(Term::NIL));
    assert_eq!(process.stack().y_reg(1), Ok(Term::NIL));
}

#[test]
fn compiled_allocate_zero_nil_initializes_slots() {
    let native = compile_body(&[
        Instruction::AllocateZero {
            stack_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(process.stack().len(), 1);
    assert_eq!(process.stack().y_reg(0), Ok(Term::NIL));
    assert_eq!(process.stack().y_reg(1), Ok(Term::NIL));
}

#[test]
fn compiled_allocate_heap_reserves_frame_after_the_heap_guard() {
    let native = compile_body(&[
        Instruction::AllocateHeap {
            stack_need: Operand::Unsigned(1),
            heap_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(process.stack().len(), 1);
    assert_eq!(process.stack().y_reg(0), Ok(Term::NIL));
}

#[test]
fn compiled_deallocate_pops_the_frame() {
    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(1),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert!(process.stack().is_empty());
}

#[test]
fn compiled_test_heap_guard_permits_execution() {
    let native = compile_body(&[
        Instruction::TestHeap {
            heap_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    let returned = call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(returned, Term::NIL.raw());
}

#[test]
fn compiled_init_yregs_nil_initializes_named_registers() {
    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::Integer(5),
            destination: Operand::Y(0),
        },
        Instruction::Move {
            source: Operand::Integer(6),
            destination: Operand::Y(1),
        },
        Instruction::InitYregs {
            registers: Operand::List(vec![Operand::Y(0), Operand::Y(1)]),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(process.stack().y_reg(0), Ok(Term::NIL));
    assert_eq!(process.stack().y_reg(1), Ok(Term::NIL));
}

#[test]
fn compiled_trim_discards_low_slots_and_renumbers_survivors() {
    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(3),
            live: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::Integer(10),
            destination: Operand::Y(0),
        },
        Instruction::Move {
            source: Operand::Integer(20),
            destination: Operand::Y(1),
        },
        Instruction::Move {
            source: Operand::Integer(30),
            destination: Operand::Y(2),
        },
        // trim words=2 remaining=1: discard the two lowest Y regs, y(2) -> y(0).
        Instruction::Trim {
            words: Operand::Unsigned(2),
            remaining: Operand::Unsigned(1),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(process.stack().current_frame().unwrap().y_slots(), 1);
    assert_eq!(process.stack().y_reg(0), Ok(Term::small_int(30)));
}

#[test]
fn compiled_test_heap_guard_survives_a_collection_with_a_live_y_register() {
    // Heap-pressure fixture (R1 acc#4): a boxed term parked in a Y register must
    // survive a collection the compiled TestHeap guard forces, with no silent
    // heap overrun. Y lives on the process stack, so the collector roots and
    // relocates it; the value read back after the guard is still correct.
    let mut process = process_with_current_module();
    let tuple = {
        let ptr = process.heap_mut().alloc(3).expect("tuple heap fits");
        // SAFETY: three contiguous heap words were just reserved for this tuple.
        let heap = unsafe { std::slice::from_raw_parts_mut(ptr, 3) };
        write_tuple(heap, &[Term::small_int(1), Term::small_int(2)]).expect("tuple layout fits")
    };
    // Fill the nursery so only a couple of words remain: the guard below must
    // therefore collect rather than satisfy the request in place.
    let free = process.heap().available();
    if free > 2 {
        process
            .heap_mut()
            .alloc(free - 2)
            .expect("nursery fill fits");
    }
    process.set_x_reg(0, tuple);

    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(1),
            live: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::Y(0),
        },
        // Needs far more than the two remaining words, forcing a collection while
        // the tuple is live only through Y(0).
        Instruction::TestHeap {
            heap_need: Operand::Unsigned(64),
            live: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        },
        Instruction::Return,
    ]);
    let returned = call_native_with_process_x_regs(&native, &mut process);

    let survived = Tuple::new(Term::from_raw(returned)).expect("Y-rooted tuple survived the GC");
    assert_eq!(survived.get(0), Some(Term::small_int(1)));
    assert_eq!(survived.get(1), Some(Term::small_int(2)));
}

#[test]
fn typed_int_moved_to_y_survives_gc_as_a_tagged_term() {
    // GC-safety pin for the typed-register X-only restriction. The typed
    // optimization keeps int values UNTAGGED in registers; a Y slot is
    // GC-rooted, so it must never hold an untagged payload (the collector would
    // trace it as a bogus term). A KNOWN-INT (typed arithmetic result) is moved
    // to a Y slot, a forced collection runs (TestHeap pressure), and the value is
    // read back: it must be the correct fully-tagged small int. Under the pre-fix
    // behavior (typed value written straight to Y) the read-back is the untagged
    // payload, not the tagged term.
    let signature = int_int_signature("y_gc");
    let native = JitCompiler::new(JitSettings)
        .unwrap()
        .compile_typed(
            &[
                // X0 = X0 + X1 (typed int path; import 0 = Add) -> untagged in X0.
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Allocate {
                    stack_need: Operand::Unsigned(1),
                    live: Operand::Unsigned(0),
                },
                // The hazard site: a typed-int X0 moved into a GC-rooted Y slot.
                Instruction::Move {
                    source: Operand::X(0),
                    destination: Operand::Y(0),
                },
                // Force a collection while Y(0) is live.
                Instruction::TestHeap {
                    heap_need: Operand::Unsigned(64),
                    live: Operand::Unsigned(1),
                },
                Instruction::Move {
                    source: Operand::Y(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            signature,
        )
        .unwrap();
    let mut process = process_with_current_module();
    // Leave the nursery nearly full so the compiled TestHeap must collect.
    let free = process.heap().available();
    if free > 2 {
        process
            .heap_mut()
            .alloc(free - 2)
            .expect("nursery fill fits");
    }
    process.set_x_reg(0, Term::small_int(5));
    process.set_x_reg(1, Term::small_int(3));
    let returned = call_native_with_process_x_regs(&native, &mut process);

    // 5 + 3 = 8, and it must come back as a fully-tagged small int.
    assert_eq!(returned, Term::small_int(8).raw());
    assert_eq!(process.stack().y_reg(0), Ok(Term::small_int(8)));
}

#[test]
fn compiled_full_frame_set_roundtrips_a_y_value() {
    // allocate -> y-reg use -> trim -> read back -> deallocate, all on the
    // stack-backed substrate. The known result doubles as a value pin; the real
    // interpreter differential on a frame-using function rides in R5.
    let native = compile_body(&[
        Instruction::Allocate {
            stack_need: Operand::Unsigned(2),
            live: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::Integer(42),
            destination: Operand::Y(0),
        },
        Instruction::Move {
            source: Operand::Integer(7),
            destination: Operand::Y(1),
        },
        // trim to the single high slot: y(1)=7 becomes y(0).
        Instruction::Trim {
            words: Operand::Unsigned(1),
            remaining: Operand::Unsigned(1),
        },
        Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(1),
        },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    let returned = call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(returned, Term::small_int(7).raw());
    assert!(process.stack().is_empty());
}

// -- JIT-002 R2: tail calls. CallLast/ApplyLast tear down the frame before an
// -- in-slice/closure transfer; CallExtLast before a cross-module transfer.

#[test]
fn compiled_call_last_self_tail_recursion_stays_flat_at_one_million() {
    // Tail-flatness pin: a self-recursive tail loop must not grow the native
    // stack per iteration. CallLast pops the frame before the in-slice jump, so a
    // 1e6-deep countdown re-allocates exactly one frame per turn and returns with
    // an empty stack. A per-iteration native frame would overflow the native
    // stack long before a million turns; completing proves the transfer is a jump.
    let native = compile_body(&[
        Instruction::Label { label: 1 },
        Instruction::Allocate {
            stack_need: Operand::Unsigned(0),
            live: Operand::Unsigned(0),
        },
        // X0 == 0 -> fall through and return; else jump to the recurse block.
        Instruction::Comparison {
            op: ComparisonOp::EqExact,
            fail: Operand::Label(2),
            left: Operand::X(0),
            right: Operand::Integer(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(0),
        },
        Instruction::Return,
        Instruction::Label { label: 2 },
        // X0 = X0 - 1 (import slot 1 = subtract).
        Instruction::Bif {
            op: BifOp::Bif2,
            operands: vec![
                Operand::Label(3),
                Operand::Unsigned(1),
                Operand::X(0),
                Operand::Integer(1),
                Operand::X(0),
            ],
        },
        Instruction::CallLast {
            arity: Operand::Unsigned(1),
            label: Operand::Label(1),
            deallocate: Operand::Unsigned(0),
        },
        Instruction::Label { label: 3 },
        Instruction::Return,
    ]);
    let mut process = process_with_current_module();
    // A budget large enough that the loop never yields on reductions.
    process.reset_reductions(3_000_000);
    process.set_x_reg(0, Term::small_int(1_000_000));
    let returned = call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(returned, Term::small_int(0).raw());
    assert!(process.stack().is_empty());
}

#[test]
fn compiled_call_ext_last_tail_calls_a_cross_module_function() {
    // CallExtLast against a genuinely external module (not a same-module alias):
    // the frame is torn down, the callee runs in another module, and its result
    // returns from the compiled body in tail position.
    let caller_atom = Atom::MODULE;
    let target_atom = Atom::ERROR;
    let function_atom = Atom::OK;
    let mut caller = test_module(
        caller_atom,
        vec![
            Instruction::Allocate {
                stack_need: Operand::Unsigned(0),
                live: Operand::Unsigned(0),
            },
            Instruction::CallExtLast {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(0),
                deallocate: Operand::Unsigned(0),
            },
        ],
    );
    caller.resolved_imports.push(ResolvedImport {
        module: target_atom,
        function: function_atom,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: target_atom,
            label: 1,
        },
    });
    let mut target = test_module(
        target_atom,
        vec![
            Instruction::Label { label: 1 },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    target.exports.insert((function_atom, 1), 1);
    let registry = ModuleRegistry::new();
    let caller = registry.insert(caller);
    let _target = registry.insert(target);
    let native = JitCompiler::new(JitSettings)
        .unwrap()
        .compile(&caller.code, caller_atom, function_atom, 1)
        .unwrap();
    let mut process = Process::new(0, 233);
    process.set_current_module(caller.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        caller.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    process.set_x_reg(0, Term::small_int(17));
    let returned = call_native_with_process_x_regs(&native, &mut process);

    // The cross-module callee echoed X0; the tail call returned it, frame popped.
    assert_eq!(returned, Term::small_int(17).raw());
    assert!(process.stack().is_empty());
}

#[test]
fn compiled_apply_last_tail_calls_a_closure() {
    let caller_atom = Atom::MODULE;
    let function_atom = Atom::OK;
    let unique_id = 0x5eed;
    let mut module = test_module(
        caller_atom,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Move {
                source: Operand::X(1),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    module
        .lambdas
        .push(test_lambda(function_atom, 1, 7, 1, unique_id));
    module.function_table.push((0, function_atom, 1));
    let registry = ModuleRegistry::new();
    let module = registry.insert(module);
    let native = JitCompiler::new(JitSettings)
        .unwrap()
        .compile(
            &[
                Instruction::Allocate {
                    stack_need: Operand::Unsigned(0),
                    live: Operand::Unsigned(0),
                },
                Instruction::ApplyLast {
                    arity: Operand::Unsigned(1),
                    deallocate: Operand::Unsigned(0),
                },
            ],
            caller_atom,
            function_atom,
            1,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let closure = heap_closure(
        &mut process,
        caller_atom,
        0,
        1,
        module.generation(),
        unique_id,
        &[Term::small_int(99)],
    );
    process.set_current_module(module.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        module.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    process.set_x_reg(0, Term::small_int(5));
    process.set_x_reg(1, closure);
    let returned = call_native_with_process_x_regs(&native, &mut process);

    // The closure returns its free var (99) after the tail frame teardown.
    assert_eq!(returned, Term::small_int(99).raw());
    assert!(process.stack().is_empty());
}

#[test]
fn compiled_swap_reads_before_writing() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Swap {
                    left: Operand::X(0),
                    right: Operand::X(1),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(2).raw(), Term::small_int(3).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(3).raw());
    assert_eq!(registers[0], Term::small_int(3).raw());
    assert_eq!(registers[1], Term::small_int(2).raw());
}

#[test]
fn compiled_add_returns_small_int_result() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::Integer(2),
                        Operand::Integer(3),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![0; 1];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(5).raw());
    assert_eq!(registers[0], Term::small_int(5).raw());
}

fn int_int_signature(name: &str) -> FunctionSignature {
    FunctionSignature {
        name: name.to_owned(),
        arity: 2,
        param_types: vec![TypeDescriptor::Int, TypeDescriptor::Int],
        return_type: TypeDescriptor::Int,
    }
}

#[test]
fn typed_add_returns_small_int_result() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = int_int_signature("add");
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            signature,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(5).raw(), Term::small_int(3).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(8).raw());
}

#[test]
fn typed_add_overflow_deopts_for_bignum_promotion() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = int_int_signature("add");
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            signature,
        )
        .unwrap();
    let mut registers = vec![
        Term::small_int(Term::SMALL_INT_MAX).raw(),
        Term::small_int(1).raw(),
    ];
    let mut process = Process::new(0, 233);
    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_DEOPT);
    assert_eq!(returned.value, JIT_DEOPT_SENTINEL as u64);
}

#[test]
fn typed_div_by_zero_takes_badarith_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = int_int_signature("div");
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(3),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            signature,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(10).raw(), Term::small_int(0).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::atom(Atom::BADARITH).raw());
}

#[test]
fn typed_mixed_known_unknown_arithmetic_materializes_known_operand_for_untyped_fallback() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "mixed_add".to_owned(),
        arity: 2,
        param_types: vec![TypeDescriptor::Int, TypeDescriptor::String],
        return_type: TypeDescriptor::String,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(99),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            signature,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(5).raw(), Term::small_int(3).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(8).raw());
}

#[test]
fn typed_div_min_by_minus_one_completes_without_overflow() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(3),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            int_int_signature("div"),
        )
        .unwrap();
    // i64::MIN as a tagged value has payload i64::MIN >> 3, which is NOT
    // i64::MIN itself — so the sdiv overflow guard (i64::MIN / -1) cannot
    // fire for valid small-int inputs. Verify it completes normally.
    let mut registers = vec![i64::MIN as u64, (-1i64) as u64];
    let returned = call_native(&native, &mut registers);

    assert_ne!(returned, JIT_DEOPT_SENTINEL as u64);
}

#[test]
fn typed_rem_by_zero_takes_badarith_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile_typed(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(4),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
            int_int_signature("rem"),
        )
        .unwrap();
    let mut registers = vec![Term::small_int(10).raw(), Term::small_int(0).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::atom(Atom::BADARITH).raw());
}

#[test]
fn compiled_add_at_end_falls_through_to_return_x0() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Label { label: 1 },
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(0),
                        Operand::Integer(2),
                        Operand::Integer(3),
                        Operand::X(0),
                    ],
                },
                Instruction::Label { label: 9 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![0; 1];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(5).raw());
}

#[test]
fn compiled_multiply_overflow_takes_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Bif {
                    op: BifOp::Bif2,
                    operands: vec![
                        Operand::Label(9),
                        Operand::Unsigned(2),
                        Operand::Integer(Term::SMALL_INT_MAX),
                        Operand::Integer(Term::SMALL_INT_MAX),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(99),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![0; 1];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(99).raw());
}

#[test]
fn compiled_branch_takes_fail_label_on_false_comparison() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Comparison {
                    op: ComparisonOp::EqExact,
                    fail: Operand::Label(7),
                    left: Operand::Integer(1),
                    right: Operand::Integer(2),
                },
                Instruction::Move {
                    source: Operand::Integer(10),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 7 },
                Instruction::Move {
                    source: Operand::Integer(20),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![0; 1];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(20).raw());
}

#[test]
fn compiled_fconv_converts_small_integer_to_float_register() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(0),
                    dest: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(42).raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let float = Float::new(Term::from_raw(registers[0])).expect("boxed float");
    assert_eq!(float.value(), 42.0);
}

#[test]
fn compiled_fconv_accepts_boxed_float() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(0),
                    dest: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let input = heap_float(&mut process, 2.75);
    let mut registers = vec![input.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let float = Float::new(Term::from_raw(registers[0])).expect("boxed float");
    assert_eq!(float.value(), 2.75);
}

#[test]
fn compiled_float_arithmetic_uses_float_registers() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fconv {
                    source: Operand::X(1),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fadd {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(0),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(2),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(2),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let left = heap_float(&mut process, 1.5);
    let right = heap_float(&mut process, 2.5);
    let mut registers = vec![left.raw(), right.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let float = Float::new(Term::from_raw(registers[0])).expect("boxed float");
    assert_eq!(float.value(), 4.0);
}

#[test]
fn compiled_fsub_fmul_and_fdiv_use_float_registers() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fconv {
                    source: Operand::X(1),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fsub {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(0),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(2),
                },
                Instruction::Fmul {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(2),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(3),
                },
                Instruction::Fdiv {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(3),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(4),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(4),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let left = heap_float(&mut process, 5.5);
    let right = heap_float(&mut process, 2.0);
    let mut registers = vec![left.raw(), right.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let float = Float::new(Term::from_raw(registers[0])).expect("boxed float");
    assert_eq!(float.value(), 3.5);
}

#[test]
fn compiled_fdiv_zero_takes_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fconv {
                    source: Operand::X(1),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fdiv {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(0),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(2),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(2),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let left = heap_float(&mut process, 1.0);
    let right = heap_float(&mut process, -0.0);
    let mut registers = vec![left.raw(), right.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::atom(Atom::BADARITH).raw());
}

#[test]
fn compiled_fdiv_positive_zero_takes_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fconv {
                    source: Operand::X(1),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fdiv {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(0),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(2),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(2),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let left = heap_float(&mut process, 1.0);
    let right = heap_float(&mut process, 0.0);
    let mut registers = vec![left.raw(), right.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::atom(Atom::BADARITH).raw());
}

#[test]
fn compiled_float_nan_or_inf_result_takes_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fconv {
                    source: Operand::X(1),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fadd {
                    fail: Operand::Label(9),
                    left: Operand::FloatRegister(0),
                    right: Operand::FloatRegister(1),
                    dest: Operand::FloatRegister(2),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(2),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let left = heap_float(&mut process, f64::MAX);
    let right = heap_float(&mut process, f64::MAX);
    let mut registers = vec![left.raw(), right.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::atom(Atom::BADARITH).raw());
}

#[test]
fn compiled_fnegate_negates_float_register() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fnegate {
                    fail: Operand::Label(9),
                    source: Operand::FloatRegister(0),
                    dest: Operand::FloatRegister(1),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(1),
                    dest: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Atom(Some(Atom::BADARITH)),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let input = heap_float(&mut process, -2.75);
    let mut registers = vec![input.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let float = Float::new(Term::from_raw(registers[0])).expect("boxed float");
    assert_eq!(float.value(), 2.75);
}

#[test]
fn compiled_float_boxing_emits_safepoint() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Fconv {
                    source: Operand::X(0),
                    dest: Operand::FloatRegister(0),
                },
                Instruction::Fmove {
                    source: Operand::FloatRegister(0),
                    dest: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0)]
    );
}

#[test]
fn compiled_put_list_emits_safepoint_and_allocates_cons() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutList {
                    head: Operand::X(0),
                    tail: Operand::Atom(None),
                    destination: Operand::X(1),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(native.stack_maps()[0].offset_from_entry, 0);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0), RootLocation::Register(1)]
    );

    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(7).raw(), Term::NIL.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(7).raw());
    let cons = Cons::new(Term::from_raw(registers[1])).unwrap();
    assert_eq!(cons.head(), Term::small_int(7));
    assert_eq!(cons.tail(), Term::NIL);
}

#[test]
fn compiled_get_list_destructures_constructed_cons() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutList {
                    head: Operand::Integer(11),
                    tail: Operand::Atom(None),
                    destination: Operand::X(1),
                },
                Instruction::GetList {
                    source: Operand::X(1),
                    head: Operand::X(2),
                    tail: Operand::X(3),
                },
                Instruction::Move {
                    source: Operand::X(2),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    let mut process = Process::new(0, 233);
    let mut registers = vec![0; 4];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(11).raw());
    assert_eq!(registers[2], Term::small_int(11).raw());
    assert_eq!(registers[3], Term::NIL.raw());
}

#[test]
fn typed_put_list_stores_tagged_int_and_typed_head_load_returns_tagged_result() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "typed_list".to_owned(),
        arity: 1,
        param_types: vec![TypeDescriptor::Int],
        return_type: TypeDescriptor::Int,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::PutList {
                    head: Operand::X(0),
                    tail: Operand::Atom(None),
                    destination: Operand::X(1),
                },
                Instruction::GetHd {
                    source: Operand::X(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
            signature,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(31).raw(), 0];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(31).raw());
    let cons = Cons::new(Term::from_raw(registers[1])).unwrap();
    assert_eq!(cons.head(), Term::small_int(31));
}

#[test]
fn compiled_get_list_read_is_pure_and_emits_no_safepoint() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::GetList {
                    source: Operand::X(1),
                    head: Operand::X(2),
                    tail: Operand::X(3),
                },
                Instruction::Move {
                    source: Operand::X(3),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert!(native.stack_maps().is_empty());
    let mut cons_words = [0; 2];
    let cons = write_cons(&mut cons_words, Term::small_int(23), Term::NIL).unwrap();
    let mut registers = vec![0, cons.raw(), 0, 0];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::NIL.raw());
    assert_eq!(registers[2], Term::small_int(23).raw());
    assert_eq!(registers[3], Term::NIL.raw());
}

#[test]
fn compiled_get_hd_and_get_tl_read_cons_fields() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutList {
                    head: Operand::Integer(17),
                    tail: Operand::X(4),
                    destination: Operand::X(1),
                },
                Instruction::GetHd {
                    source: Operand::X(1),
                    destination: Operand::X(2),
                },
                Instruction::GetTl {
                    source: Operand::X(1),
                    destination: Operand::X(3),
                },
                Instruction::Move {
                    source: Operand::X(3),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    let mut process = Process::new(0, 233);
    let mut registers = vec![0, 0, 0, 0, Term::NIL.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::NIL.raw());
    assert_eq!(registers[2], Term::small_int(17).raw());
    assert_eq!(registers[3], Term::NIL.raw());
}

#[test]
fn compiled_get_hd_and_get_tl_reads_are_pure_and_emit_no_safepoint() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::GetHd {
                    source: Operand::X(1),
                    destination: Operand::X(2),
                },
                Instruction::GetTl {
                    source: Operand::X(1),
                    destination: Operand::X(3),
                },
                Instruction::Move {
                    source: Operand::X(2),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert!(native.stack_maps().is_empty());
    let mut cons_words = [0; 2];
    let cons = write_cons(&mut cons_words, Term::small_int(29), Term::NIL).unwrap();
    let mut registers = vec![0, cons.raw(), 0, 0];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(29).raw());
    assert_eq!(registers[2], Term::small_int(29).raw());
    assert_eq!(registers[3], Term::NIL.raw());
}

#[test]
fn compiled_put_tuple2_emits_safepoint_and_allocates_tuple() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutTuple2 {
                    destination: Operand::X(2),
                    elements: Operand::List(vec![Operand::X(0), Operand::Integer(9)]),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(native.stack_maps()[0].offset_from_entry, 0);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0), RootLocation::Register(2)]
    );

    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(4).raw(), 0, 0];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(4).raw());
    let tuple = Tuple::new(Term::from_raw(registers[2])).unwrap();
    assert_eq!(tuple.arity(), 2);
    assert_eq!(tuple.get(0), Some(Term::small_int(4)));
    assert_eq!(tuple.get(1), Some(Term::small_int(9)));
}

#[test]
fn compiled_put_map_assoc_emits_safepoint_and_allocates_map() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::MapOp {
                    op: MapOp::PutMapAssoc,
                    operands: vec![
                        Operand::Label(1),
                        Operand::X(1),
                        Operand::X(4),
                        Operand::Unsigned(0),
                        Operand::List(vec![
                            Operand::X(0),
                            Operand::X(2),
                            Operand::Atom(Some(Atom::ERROR)),
                            Operand::X(3),
                        ]),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 1 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(native.stack_maps()[0].offset_from_entry, 0);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![
            RootLocation::Register(1),
            RootLocation::Register(0),
            RootLocation::Register(2),
            RootLocation::Register(3),
            RootLocation::Register(4),
        ]
    );

    let mut process = Process::new(0, 233);
    let empty = heap_map(&mut process, &[]);
    let mut registers = vec![
        Term::atom(Atom::OK).raw(),
        empty.raw(),
        Term::small_int(1).raw(),
        Term::small_int(2).raw(),
        0,
    ];
    let _returned = call_native_with_process(&native, &mut registers, &mut process);

    let map = Map::new(Term::from_raw(registers[4])).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(Term::atom(Atom::OK)), Some(Term::small_int(1)));
    assert_eq!(map.get(Term::atom(Atom::ERROR)), Some(Term::small_int(2)));
}

#[test]
fn compiled_put_map_assoc_allocates_empty_map() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::MapOp {
                    op: MapOp::PutMapAssoc,
                    operands: vec![
                        Operand::Label(1),
                        Operand::X(1),
                        Operand::X(0),
                        Operand::Unsigned(0),
                        Operand::List(vec![]),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 1 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let empty = heap_map(&mut process, &[]);
    let mut registers = vec![0, empty.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, registers[0]);
    let map = Map::new(Term::from_raw(registers[0])).unwrap();
    assert!(map.is_empty());
}

#[test]
fn compiled_put_map_exact_updates_existing_key_and_fails_missing_key() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::MapOp {
                    op: MapOp::PutMapExact,
                    operands: vec![
                        Operand::Label(1),
                        Operand::X(1),
                        Operand::X(0),
                        Operand::Unsigned(0),
                        Operand::List(vec![Operand::Atom(Some(Atom::OK)), Operand::X(2)]),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 1 },
                Instruction::Move {
                    source: Operand::X(3),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    let mut process = Process::new(0, 233);
    let source = heap_map(&mut process, &[(Term::atom(Atom::OK), Term::small_int(1))]);
    let mut registers = vec![0, source.raw(), Term::small_int(2).raw(), Term::NIL.raw()];
    let _returned = call_native_with_process(&native, &mut registers, &mut process);
    let updated = Map::new(Term::from_raw(registers[0])).unwrap();
    assert_eq!(updated.get(Term::atom(Atom::OK)), Some(Term::small_int(2)));
    assert_eq!(
        Map::new(source).unwrap().get(Term::atom(Atom::OK)),
        Some(Term::small_int(1))
    );

    let missing_source = heap_map(
        &mut process,
        &[(Term::atom(Atom::ERROR), Term::small_int(1))],
    );
    let mut registers = vec![
        0,
        missing_source.raw(),
        Term::small_int(3).raw(),
        Term::NIL.raw(),
    ];
    let returned = call_native_with_process(&native, &mut registers, &mut process);
    assert_eq!(returned, Term::NIL.raw());
}

#[test]
fn compiled_map_pattern_ops_are_read_only_and_emit_no_safepoints() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::MapOp {
                    op: MapOp::HasMapFields,
                    operands: vec![
                        Operand::Label(1),
                        Operand::X(1),
                        Operand::List(vec![Operand::Atom(Some(Atom::OK))]),
                    ],
                },
                Instruction::MapOp {
                    op: MapOp::GetMapElements,
                    operands: vec![
                        Operand::Label(1),
                        Operand::X(1),
                        Operand::List(vec![Operand::Atom(Some(Atom::OK)), Operand::X(0)]),
                    ],
                },
                Instruction::Return,
                Instruction::Label { label: 1 },
                Instruction::Move {
                    source: Operand::X(2),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert!(native.stack_maps().is_empty());
    let mut process = Process::new(0, 233);
    let source = heap_map(&mut process, &[(Term::atom(Atom::OK), Term::small_int(42))]);
    let mut registers = vec![0, source.raw(), Term::NIL.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);
    assert_eq!(returned, Term::small_int(42).raw());

    let missing = heap_map(
        &mut process,
        &[(Term::atom(Atom::ERROR), Term::small_int(42))],
    );
    let mut registers = vec![0, missing.raw(), Term::NIL.raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);
    assert_eq!(returned, Term::NIL.raw());
}

#[test]
fn typed_put_tuple_stores_tagged_int_and_typed_element_load_returns_tagged_result() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "typed_tuple".to_owned(),
        arity: 1,
        param_types: vec![TypeDescriptor::Int],
        return_type: TypeDescriptor::Int,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::PutTuple2 {
                    destination: Operand::X(1),
                    elements: Operand::List(vec![Operand::X(0)]),
                },
                Instruction::GetTupleElement {
                    source: Operand::X(1),
                    index: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
            signature,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(41).raw(), 0];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(41).raw());
    let tuple = Tuple::new(Term::from_raw(registers[1])).unwrap();
    assert_eq!(tuple.get(0), Some(Term::small_int(41)));
}

#[test]
fn compiled_get_tuple_element_reads_constructed_tuple() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutTuple2 {
                    destination: Operand::X(1),
                    elements: Operand::List(vec![Operand::Integer(4), Operand::Integer(9)]),
                },
                Instruction::GetTupleElement {
                    source: Operand::X(1),
                    index: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    let mut process = Process::new(0, 233);
    let mut registers = vec![0; 2];
    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(9).raw());
    let tuple = Tuple::new(Term::from_raw(registers[1])).unwrap();
    assert_eq!(tuple.get(0), Some(Term::small_int(4)));
    assert_eq!(tuple.get(1), Some(Term::small_int(9)));
}

#[test]
fn compiled_allocation_with_tiny_heap_survives_gc() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::PutTuple2 {
                    destination: Operand::X(1),
                    elements: Operand::List(vec![Operand::X(0)]),
                },
                Instruction::PutTuple2 {
                    destination: Operand::X(2),
                    elements: Operand::List(vec![Operand::X(1), Operand::Integer(8)]),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 2);

    let mut process = Process::new(0, 2);
    process.set_x_reg(0, Term::small_int(3));
    let returned = call_native_with_process_x_regs(&native, &mut process);

    assert_eq!(returned, Term::small_int(3).raw());
    let outer = Tuple::new(process.x_reg(2)).unwrap();
    assert_eq!(outer.arity(), 2);
    let inner = Tuple::new(outer.get(0).unwrap()).unwrap();
    assert_eq!(inner.get(0), Some(Term::small_int(3)));
    assert_eq!(outer.get(1), Some(Term::small_int(8)));
}

#[test]
fn compiled_make_fun_captures_two_free_vars_and_records_safepoint_roots() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let lambdas = vec![test_lambda(Atom::OK, 1, 7, 2, 0xfeed)];
    let native = compiler
        .compile_module_function(
            &[
                Instruction::MakeFun {
                    operands: vec![Operand::Unsigned(0)],
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
            ModuleCompileMetadata {
                lambdas: &lambdas,
                generation: 9,
            },
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(native.stack_maps()[0].offset_from_entry, 0);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0), RootLocation::Register(1),]
    );

    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::small_int(11).raw(), Term::atom(Atom::ERROR).raw()];
    let returned = call_native_with_process(&native, &mut registers, &mut process);
    let closure = Closure::new(Term::from_raw(returned)).expect("closure term");

    assert_eq!(registers[0], returned);
    assert_eq!(closure.module(), Some(Atom::MODULE));
    assert_eq!(closure.function_index(), 0);
    assert_eq!(closure.arity(), 1);
    assert_eq!(closure.num_free(), 2);
    assert_eq!(closure.generation(), 9);
    assert_eq!(closure.unique_id(), 0xfeed);
    assert_eq!(closure.free_var(0), Some(Term::small_int(11)));
    assert_eq!(closure.free_var(1), Some(Term::atom(Atom::ERROR)));
}

#[test]
fn compiled_make_fun_creates_valid_zero_capture_closure() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let lambdas = vec![test_lambda(Atom::OK, 0, 7, 0, 0xbeef)];
    let native = compiler
        .compile_module_function(
            &[
                Instruction::MakeFun {
                    operands: vec![
                        Operand::Unsigned(0),
                        Operand::Unsigned(0),
                        Operand::Unsigned(0),
                    ],
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
            ModuleCompileMetadata {
                lambdas: &lambdas,
                generation: 4,
            },
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0)]
    );

    let mut process = Process::new(0, 233);
    let mut registers = vec![0];
    let returned = call_native_with_process(&native, &mut registers, &mut process);
    let closure = Closure::new(Term::from_raw(returned)).expect("closure term");

    assert_eq!(closure.module(), Some(Atom::MODULE));
    assert_eq!(closure.arity(), 0);
    assert_eq!(closure.num_free(), 0);
    assert_eq!(closure.free_var(0), None);
}

#[test]
fn compiled_call_fun_copies_free_vars_after_explicit_args_and_returns_value() {
    let caller_atom = Atom::MODULE;
    let function_atom = Atom::OK;
    let unique_id = 0xabcddcba;
    let mut module = test_module(
        caller_atom,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Move {
                source: Operand::X(1),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    module
        .lambdas
        .push(test_lambda(function_atom, 1, 7, 1, unique_id));
    module.function_table.push((0, function_atom, 1));
    let registry = ModuleRegistry::new();
    let module = registry.insert(module);
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            // Tail-position wall (R3): the call is admitted only immediately
            // before a Return (the lowering already returns the callee's result).
            &[
                Instruction::CallFun {
                    arity: Operand::Unsigned(1),
                },
                Instruction::Return,
            ],
            caller_atom,
            function_atom,
            1,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let closure = heap_closure(
        &mut process,
        caller_atom,
        0,
        1,
        module.generation(),
        unique_id,
        &[Term::small_int(99)],
    );
    process.set_current_module(module.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        module.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![Term::small_int(5).raw(), closure.raw()];

    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(99).raw());
    assert_eq!(process.x_reg(0), Term::small_int(99));
    assert_eq!(process.x_reg(1), Term::small_int(99));
}

#[test]
fn compiled_call_fun2_dispatches_the_admitted_lowering() {
    // JIT-002 R3: CallFun2's lowering already existed but the pre-pass rejected
    // it, leaving it dead. With CallFun2 admitted, a CallFun2-bearing function
    // compiles and dispatches the closure, result equal to the interpreter's.
    let caller_atom = Atom::MODULE;
    let function_atom = Atom::OK;
    let unique_id = 0xc0ffee;
    let mut module = test_module(
        caller_atom,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Move {
                source: Operand::X(1),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    module
        .lambdas
        .push(test_lambda(function_atom, 1, 7, 1, unique_id));
    module.function_table.push((0, function_atom, 1));
    let registry = ModuleRegistry::new();
    let module = registry.insert(module);
    let native = JitCompiler::new(JitSettings)
        .unwrap()
        .compile(
            // Tail-position wall (R3): CallFun2 admitted immediately before Return.
            &[
                Instruction::CallFun2 {
                    function: Operand::X(1),
                    arity: Operand::Unsigned(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            caller_atom,
            function_atom,
            1,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let closure = heap_closure(
        &mut process,
        caller_atom,
        0,
        1,
        module.generation(),
        unique_id,
        &[Term::small_int(99)],
    );
    process.set_current_module(module.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        module.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![Term::small_int(5).raw(), closure.raw()];

    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(99).raw());
}

#[test]
fn compiled_apply_wrong_arity_raises_badarity() {
    let caller_atom = Atom::MODULE;
    let function_atom = Atom::OK;
    let unique_id = 0x1234;
    let mut module = test_module(
        caller_atom,
        vec![Instruction::Label { label: 7 }, Instruction::Return],
    );
    module
        .lambdas
        .push(test_lambda(function_atom, 2, 7, 0, unique_id));
    let registry = ModuleRegistry::new();
    let module = registry.insert(module);
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            // Tail-position wall (R3): Apply admitted immediately before Return.
            &[
                Instruction::Apply {
                    arity: Operand::Unsigned(1),
                },
                Instruction::Return,
            ],
            caller_atom,
            function_atom,
            1,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let closure = heap_closure(
        &mut process,
        caller_atom,
        0,
        2,
        module.generation(),
        unique_id,
        &[],
    );
    process.set_current_module(module.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        module.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![Term::small_int(5).raw(), closure.raw()];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_EXCEPTION);
    assert_eq!(returned.value, Term::atom(Atom::BADARITY).raw());
    let exception = process
        .current_exception()
        .expect("badarity exception state");
    assert_eq!(exception.class, Term::atom(Atom::ERROR));
    assert_eq!(exception.reason, Term::atom(Atom::BADARITY));
}

#[test]
fn compiled_make_fun_survives_gc_before_apply() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let lambdas = vec![test_lambda(Atom::OK, 0, 7, 1, 0xcafe)];
    let native = compiler
        .compile_module_function(
            &[
                Instruction::MakeFun {
                    operands: vec![Operand::Unsigned(0)],
                },
                Instruction::PutTuple2 {
                    destination: Operand::X(2),
                    elements: Operand::List(vec![Operand::X(0)]),
                },
                // Tail-position wall (R3): CallFun admitted immediately before Return.
                Instruction::CallFun {
                    arity: Operand::Unsigned(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
            ModuleCompileMetadata {
                lambdas: &lambdas,
                generation: 0,
            },
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 2);
    assert_eq!(
        native.stack_maps()[1].live_roots,
        vec![RootLocation::Register(0), RootLocation::Register(2)]
    );
}

#[test]
fn compiled_is_integer_distinguishes_integer_from_atom() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::TypeTest {
                    op: TypeTestOp::IsInteger,
                    fail: Operand::Label(7),
                    value: Operand::X(0),
                },
                Instruction::Move {
                    source: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 7 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    let mut integer_registers = vec![Term::small_int(42).raw()];
    assert_eq!(
        call_native(&native, &mut integer_registers),
        Term::small_int(1).raw()
    );
    let mut atom_registers = vec![Term::atom(Atom::OK).raw()];
    assert_eq!(
        call_native(&native, &mut atom_registers),
        Term::small_int(0).raw()
    );
}

#[test]
fn typed_is_integer_guard_elides_known_int_and_returns_tagged_value() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "guard".to_owned(),
        arity: 1,
        param_types: vec![TypeDescriptor::Int],
        return_type: TypeDescriptor::Int,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::TypeTest {
                    op: TypeTestOp::IsInteger,
                    fail: Operand::Label(7),
                    value: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 7 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
            signature,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(55).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(55).raw());
}

#[test]
fn typed_is_atom_guard_on_known_int_jumps_to_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "guard".to_owned(),
        arity: 1,
        param_types: vec![TypeDescriptor::Int],
        return_type: TypeDescriptor::Int,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::TypeTest {
                    op: TypeTestOp::IsAtom,
                    fail: Operand::Label(7),
                    value: Operand::X(0),
                },
                Instruction::Move {
                    source: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 7 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
            signature,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(55).raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(0).raw());
}

#[test]
fn typed_test_arity_uses_known_tuple_length() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let signature = FunctionSignature {
        name: "arity".to_owned(),
        arity: 1,
        param_types: vec![TypeDescriptor::Tuple(vec![
            TypeDescriptor::Int,
            TypeDescriptor::Int,
        ])],
        return_type: TypeDescriptor::Int,
    };
    let native = compiler
        .compile_typed(
            &[
                Instruction::TestArity {
                    fail: Operand::Label(7),
                    tuple: Operand::X(0),
                    arity: Operand::Integer(3),
                },
                Instruction::Move {
                    source: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 7 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
            signature,
        )
        .unwrap();
    let mut tuple_words = [0; 3];
    let tuple = write_tuple(&mut tuple_words, &[Term::small_int(1), Term::small_int(2)]).unwrap();
    let mut registers = vec![tuple.raw()];
    let returned = call_native(&native, &mut registers);

    assert_eq!(returned, Term::small_int(0).raw());
}

#[test]
fn compiled_pattern_match_on_ok_tuple() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::IsTaggedTuple {
                    fail: Operand::Label(9),
                    value: Operand::X(0),
                    arity: Operand::Unsigned(2),
                    tag: Operand::Atom(Some(Atom::OK)),
                },
                Instruction::Move {
                    source: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut tuple_words = [0; 3];
    let tuple = write_tuple(
        &mut tuple_words,
        &[Term::atom(Atom::OK), Term::small_int(42)],
    )
    .unwrap();
    let mut registers = vec![tuple.raw()];

    assert_eq!(
        call_native(&native, &mut registers),
        Term::small_int(1).raw()
    );
}

#[test]
fn compiled_select_val_dispatches_matching_atom() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::SelectVal {
                    value: Operand::X(0),
                    fail: Operand::Label(9),
                    list: Operand::List(vec![
                        Operand::Atom(Some(Atom::OK)),
                        Operand::Label(2),
                        Operand::Integer(7),
                        Operand::Label(3),
                    ]),
                },
                Instruction::Label { label: 2 },
                Instruction::Move {
                    source: Operand::Integer(20),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 3 },
                Instruction::Move {
                    source: Operand::Integer(30),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(90),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![Term::atom(Atom::OK).raw()];

    assert_eq!(
        call_native(&native, &mut registers),
        Term::small_int(20).raw()
    );
}

#[test]
fn compiled_select_val_does_not_fall_through_after_match() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::SelectVal {
                    value: Operand::X(0),
                    fail: Operand::Label(9),
                    list: Operand::List(vec![Operand::Integer(7), Operand::Label(2)]),
                },
                Instruction::Move {
                    source: Operand::Integer(99),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 2 },
                Instruction::Move {
                    source: Operand::Integer(20),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(90),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut registers = vec![Term::small_int(7).raw()];

    assert_eq!(
        call_native(&native, &mut registers),
        Term::small_int(20).raw()
    );
}

#[test]
fn compiled_zero_arity_is_tagged_tuple_takes_fail_label() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::IsTaggedTuple {
                    fail: Operand::Label(9),
                    value: Operand::X(0),
                    arity: Operand::Unsigned(0),
                    tag: Operand::Atom(Some(Atom::OK)),
                },
                Instruction::Move {
                    source: Operand::Integer(1),
                    destination: Operand::X(0),
                },
                Instruction::Return,
                Instruction::Label { label: 9 },
                Instruction::Move {
                    source: Operand::Integer(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut tuple_words = [0; 1];
    let tuple = write_tuple(&mut tuple_words, &[]).unwrap();
    let mut registers = vec![tuple.raw()];

    assert_eq!(
        call_native(&native, &mut registers),
        Term::small_int(0).raw()
    );
}

#[test]
fn compiled_external_call_falls_back_to_interpreter_and_returns_value() {
    let caller_atom = Atom::MODULE;
    let target_atom = Atom::ERROR;
    let function_atom = Atom::OK;
    let mut caller = test_module(
        caller_atom,
        vec![Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        }],
    );
    caller.resolved_imports.push(ResolvedImport {
        module: target_atom,
        function: function_atom,
        arity: 1,
        target: ResolvedImportTarget::Code {
            module: target_atom,
            label: 1,
        },
    });
    let mut target = test_module(
        target_atom,
        vec![
            Instruction::Label { label: 1 },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    target.exports.insert((function_atom, 1), 1);
    let registry = ModuleRegistry::new();
    let caller = registry.insert(caller);
    let _target = registry.insert(target);
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(&caller.code, caller_atom, function_atom, 1)
        .unwrap();
    let mut process = Process::new(0, 233);
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        caller.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![Term::small_int(17).raw()];

    let returned = call_native_with_process(&native, &mut registers, &mut process);

    assert_eq!(returned, Term::small_int(17).raw());
    assert_eq!(registers[0], Term::small_int(17).raw());
}

#[test]
fn compiled_try_catches_interpreted_exception_and_exposes_payload() {
    let caller_atom = Atom::MODULE;
    let target_atom = Atom::ERROR;
    let function_atom = Atom::OK;
    let mut caller = test_module(
        caller_atom,
        vec![
            // Re-pinned for JIT-002 R1: the Try/TryCase Y triplet now lives on the
            // process call stack, so the compiled body reserves it with `allocate`
            // (three Y slots) instead of relying on the removed flat-Y buffer.
            Instruction::Allocate {
                stack_need: Operand::Unsigned(3),
                live: Operand::Unsigned(0),
            },
            Instruction::Try {
                destination: Operand::Y(0),
                label: Operand::Label(20),
            },
            Instruction::Move {
                source: Operand::Atom(Some(Atom::ERROR)),
                destination: Operand::X(0),
            },
            Instruction::Move {
                source: Operand::Atom(Some(Atom::BADARG)),
                destination: Operand::X(1),
            },
            Instruction::Move {
                source: Operand::Atom(None),
                destination: Operand::X(2),
            },
            Instruction::CallExtOnly {
                arity: Operand::Unsigned(3),
                import: Operand::Unsigned(0),
            },
            Instruction::Label { label: 20 },
            Instruction::TryCase {
                source: Operand::Y(0),
            },
            Instruction::Return,
        ],
    );
    caller.resolved_imports.push(ResolvedImport {
        module: target_atom,
        function: function_atom,
        arity: 3,
        target: ResolvedImportTarget::Code {
            module: target_atom,
            label: 1,
        },
    });
    let mut target = test_module(
        target_atom,
        vec![
            Instruction::Label { label: 1 },
            Instruction::RawRaise,
            Instruction::Return,
        ],
    );
    target.exports.insert((function_atom, 3), 1);
    let registry = ModuleRegistry::new();
    let caller = registry.insert(caller);
    let _target = registry.insert(target);
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(&caller.code, caller_atom, function_atom, 0)
        .unwrap();
    let mut process = Process::new(0, 233);
    process.set_current_module(caller.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        caller.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![0; X_REGISTER_COUNT as usize + 3];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_NORMAL);
    assert_eq!(returned.value, Term::atom(Atom::ERROR).raw());
    assert_eq!(registers[0], Term::atom(Atom::ERROR).raw());
    assert_eq!(registers[1], Term::atom(Atom::BADARG).raw());
    assert_eq!(registers[2], Term::NIL.raw());
    assert_eq!(process.current_exception(), None);
}

#[test]
fn compiled_external_exception_without_try_propagates_status_and_frame() {
    let caller_atom = Atom::MODULE;
    let target_atom = Atom::ERROR;
    let function_atom = Atom::OK;
    let mut caller = test_module(
        caller_atom,
        vec![Instruction::CallExtOnly {
            arity: Operand::Unsigned(3),
            import: Operand::Unsigned(0),
        }],
    );
    caller.resolved_imports.push(ResolvedImport {
        module: target_atom,
        function: function_atom,
        arity: 3,
        target: ResolvedImportTarget::Code {
            module: target_atom,
            label: 1,
        },
    });
    let mut target = test_module(
        target_atom,
        vec![
            Instruction::Label { label: 1 },
            Instruction::RawRaise,
            Instruction::Return,
        ],
    );
    target.exports.insert((function_atom, 3), 1);
    let registry = ModuleRegistry::new();
    let caller = registry.insert(caller);
    let _target = registry.insert(target);
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(&caller.code, caller_atom, function_atom, 0)
        .unwrap();
    let mut process = Process::new(0, 233);
    process.set_current_module(caller.clone());
    process.set_jit_runtime_context(Some(JitRuntimeContext::new(
        caller.as_ref() as *const Module,
        &registry as *const ModuleRegistry,
        std::ptr::null(),
    )));
    let mut registers = vec![
        Term::atom(Atom::ERROR).raw(),
        Term::atom(Atom::BADARG).raw(),
        Term::NIL.raw(),
    ];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_EXCEPTION);
    assert_eq!(returned.value, Term::atom(Atom::BADARG).raw());
    let exception = process
        .current_exception()
        .expect("exception state preserved");
    assert_eq!(exception.class, Term::atom(Atom::ERROR));
    assert_eq!(exception.reason, Term::atom(Atom::BADARG));
    assert!(
        process
            .raw_stacktrace()
            .iter()
            .any(|entry| entry.mfa == Some((caller_atom, function_atom, 0)))
    );
}

#[test]
fn compiled_local_call_charges_reduction_and_yields_when_exhausted() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::Label { label: 1 },
                Instruction::CallOnly {
                    arity: Operand::Unsigned(0),
                    label: Operand::Label(1),
                },
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    process.reset_reductions(3);
    let mut registers = vec![0];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_YIELD);
    assert_eq!(
        returned.value,
        crate::jit::runtime::JIT_YIELD_SENTINEL as u64,
    );
    assert_eq!(process.reduction_counter(), 0);
}

#[test]
fn compiled_external_call_returns_deopt_sentinel_without_runtime_context() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[Instruction::CallExtOnly {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            }],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let mut registers = vec![0];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_DEOPT);
    assert_eq!(returned.value, JIT_DEOPT_SENTINEL as u64);
    assert_eq!(registers[0], 0);
}

#[test]
fn selective_receive_peek_lowers_and_send_then_receive_is_walled() {
    let compiler = JitCompiler::new(JitSettings).unwrap();

    // The receive PEEK/accept lowering (loop_rec / remove_message / loop_rec_end)
    // compiles in a sound arrangement: the first loop_rec has no observable side
    // effect before it, and no deopt-capable op follows the accept.
    compiler
        .compile(
            &[
                Instruction::Label { label: 1 },
                Instruction::LoopRec {
                    fail: Operand::Label(2),
                    destination: Operand::X(0),
                },
                Instruction::RemoveMessage,
                Instruction::Return,
                Instruction::LoopRecEnd {
                    fail: Operand::Label(1),
                },
                Instruction::Label { label: 2 },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    // LEG 1b wall: a Send followed by the receive peek (loop_rec is deopt-capable)
    // is rejected — a deopt at the peek would replay the Send.
    let send_then_receive = compiler.compile(
        &[
            Instruction::Send,
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        2,
    );
    assert!(
        matches!(send_then_receive, Err(JitError::UnsupportedOpcode { .. })),
        "Send-then-receive is deopt-after-side-effect and must be walled: {send_then_receive:?}"
    );
}

#[test]
fn compiled_send_records_destination_and_message_safepoint_roots() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[Instruction::Send, Instruction::Return],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();

    assert_eq!(native.stack_maps().len(), 1);
    assert_eq!(native.stack_maps()[0].offset_from_entry, 0);
    assert_eq!(
        native.stack_maps()[0].live_roots,
        vec![RootLocation::Register(0), RootLocation::Register(1)]
    );
}

#[test]
fn wait_timeout_and_blocking_receive_lower_under_path_sensitivity() {
    let compiler = JitCompiler::new(JitSettings).unwrap();

    // wait_timeout / timeout lower in a sound arrangement (no observable side
    // effect precedes the deopt-capable wait_timeout).
    compiler
        .compile(
            &[
                Instruction::Label { label: 1 },
                Instruction::WaitTimeout {
                    fail: Operand::Label(2),
                    timeout: Operand::Unsigned(0),
                },
                Instruction::Label { label: 2 },
                Instruction::Timeout,
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap();

    // CFG-SENSITIVE UN-WALLING: a real blocking receive places wait/wait_timeout
    // after loop_rec, but the peek/park are PURE (not observable side effects) and
    // no observable effect is reachable on the path to the deopt edges — so the
    // whole blocking-receive shape now ADMITS. (Under the old linear guard this
    // was walled; the path-true dataflow admits it. RemoveMessage — the real loss
    // effect — is absent here; see `two_sequential_receives_...` for the loss
    // path that stays rejected.)
    compiler
        .compile(
            &[
                Instruction::Label { label: 1 },
                Instruction::LoopRec {
                    fail: Operand::Label(2),
                    destination: Operand::X(0),
                },
                Instruction::LoopRecEnd {
                    fail: Operand::Label(1),
                },
                Instruction::Label { label: 2 },
                Instruction::WaitTimeout {
                    fail: Operand::Label(3),
                    timeout: Operand::Unsigned(0),
                },
                Instruction::Label { label: 3 },
                Instruction::Timeout,
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .expect("a blocking receive with no consume effect admits under path-sensitivity");
}

#[test]
fn recv_marker_opcodes_compile_to_deopt() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let native = compiler
        .compile(
            &[
                Instruction::RecvMarkerUse {
                    marker: Operand::X(0),
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            1,
        )
        .unwrap();
    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::NIL.raw()];

    let returned = call_native_status(&native, &mut registers, &mut process);

    assert_eq!(returned.status, JIT_STATUS_DEOPT);
    assert_eq!(returned.value, JIT_DEOPT_SENTINEL as u64);
}

#[test]
fn reports_unsupported_opcode() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let error = compiler
        .compile(
            &[Instruction::Generic {
                opcode: 255,
                name: "unknown",
                operands: Vec::new(),
            }],
            Atom::MODULE,
            Atom::OK,
            0,
        )
        .unwrap_err();

    assert_eq!(
        error,
        JitError::UnsupportedOpcode {
            opcode: "unknown (255)".to_owned()
        }
    );
}

// -- JIT-002 R3: the tail-position wall. The tier has no body-call model, so a
// -- body-position CallExt/Apply/CallFun/CallFun2/Call would silently drop its
// -- continuation. The pre-pass rejects the whole function; the mis-compile is
// -- unreachable. This is the 777 probe (a body CallExt followed by Move 777),
// -- armed as a wall.

#[test]
fn tail_position_wall_rejects_every_body_position_call() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    // The 777 probe: a body CallExt whose continuation (Move 777 -> X0) the
    // tail-only lowering would silently drop. Plus the other four call forms in
    // body position. Each must be rejected as an unsupported opcode by the wall.
    let body_position_slices = [
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(0),
            },
            Instruction::Move {
                source: Operand::Integer(777),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        vec![
            Instruction::Apply {
                arity: Operand::Unsigned(1),
            },
            Instruction::Move {
                source: Operand::Integer(777),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        vec![
            Instruction::CallFun {
                arity: Operand::Unsigned(1),
            },
            Instruction::Move {
                source: Operand::Integer(777),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        vec![
            Instruction::CallFun2 {
                function: Operand::X(1),
                arity: Operand::Unsigned(1),
                destination: Operand::X(0),
            },
            Instruction::Move {
                source: Operand::Integer(777),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        vec![
            Instruction::Call {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
            Instruction::Move {
                source: Operand::Integer(777),
                destination: Operand::X(0),
            },
            Instruction::Return,
            Instruction::Label { label: 1 },
            Instruction::Return,
        ],
    ];
    for slice in &body_position_slices {
        let result = compiler.compile(slice, Atom::MODULE, Atom::OK, 1);
        assert!(
            matches!(result, Err(JitError::UnsupportedOpcode { .. })),
            "body-position call must be rejected by the tail-position wall, got {result:?} for {slice:?}",
        );
    }
}

#[test]
fn tail_position_wall_admits_a_tail_external_call() {
    // The positive control: the SAME CallExt immediately before Return clears the
    // wall (the *Only/*Last forms and tail CallExt are the legal shape).
    let native = JitCompiler::new(JitSettings).unwrap().compile(
        &[
            Instruction::CallExt {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        1,
    );
    assert!(
        native.is_ok(),
        "a tail-position CallExt must compile: {native:?}"
    );
}

// -- JIT-002 R3 BIF NO-FAIL RULING: erlc emits {f,0} (no local handler) on all
// -- body-position arithmetic. It routes to the deopt block under a structural
// -- purity guard (no observable side effect may precede it, since deopt restarts
// -- the callee interpreted from its start).

#[test]
fn no_fail_bif_computes_small_ints_and_deopts_on_badarith() {
    // {f,0} arithmetic routes to the deopt block: valid small ints compute in
    // native; a non-small-int operand takes the deopt edge (JIT_STATUS_DEOPT ->
    // Ok(None) -> the interpreter re-runs and raises the same badarith). GcBif2
    // 6-operand form, import 1 = Subtract.
    let native = JitCompiler::new(JitSettings)
        .unwrap()
        .compile(
            &[
                Instruction::Bif {
                    op: BifOp::GcBif2,
                    operands: vec![
                        Operand::Label(0),
                        Operand::Unsigned(0),
                        Operand::Unsigned(1),
                        Operand::X(0),
                        Operand::X(1),
                        Operand::X(0),
                    ],
                },
                Instruction::Return,
            ],
            Atom::MODULE,
            Atom::OK,
            2,
        )
        .unwrap();
    // Valid: 5 - 3 = 2, no deopt.
    let mut registers = vec![Term::small_int(5).raw(), Term::small_int(3).raw()];
    assert_eq!(
        call_native(&native, &mut registers),
        Term::small_int(2).raw()
    );
    // Badarith: an atom operand can't be a small int -> deopt to the interpreter.
    let mut process = Process::new(0, 233);
    let mut registers = vec![Term::atom(Atom::OK).raw(), Term::small_int(3).raw()];
    let returned = call_native_status(&native, &mut registers, &mut process);
    assert_eq!(
        returned.status, JIT_STATUS_DEOPT,
        "a non-small-int operand routes {{f,0}} arithmetic to the deopt fallback"
    );
}

#[test]
fn deopt_capable_op_after_a_side_effect_is_rejected_by_the_guard() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    // Deopt restarts the callee from its start, so a {f,0} Bif after a Send would
    // re-send on restart — the LEG 1b guard rejects the whole function.
    let after_send = compiler.compile(
        &[
            Instruction::Send,
            Instruction::Bif {
                op: BifOp::GcBif2,
                operands: vec![
                    Operand::Label(0),
                    Operand::Unsigned(0),
                    Operand::Unsigned(1),
                    Operand::X(0),
                    Operand::X(1),
                    Operand::X(0),
                ],
            },
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        2,
    );
    assert!(
        matches!(after_send, Err(JitError::UnsupportedOpcode { .. })),
        "a {{f,0}} Bif after a Send must be rejected, got {after_send:?}"
    );
    // LEG 1b WIDENS the JIT-002 {f,0}-only guard: a REAL-fail-label Bif after a
    // Send is ALSO rejected now, because the typed-overflow route branches to the
    // deopt block regardless of the fail label (SCOPING §2's un-guarded gap) — the
    // Bif is deopt-capable, so a deopt-restart could still re-send.
    let real_fail_after_send = compiler.compile(
        &[
            Instruction::Send,
            Instruction::Bif {
                op: BifOp::GcBif2,
                operands: vec![
                    Operand::Label(9),
                    Operand::Unsigned(0),
                    Operand::Unsigned(1),
                    Operand::X(0),
                    Operand::X(1),
                    Operand::X(0),
                ],
            },
            Instruction::Return,
            Instruction::Label { label: 9 },
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        2,
    );
    assert!(
        matches!(
            real_fail_after_send,
            Err(JitError::UnsupportedOpcode { .. })
        ),
        "a real-fail-label Bif after a Send is deopt-capable (typed overflow) and \
         must be rejected: {real_fail_after_send:?}"
    );
    // POSITIVE CONTROL: the SAME deopt-capable Bif BEFORE the side effect is
    // admitted — nothing observable precedes the deopt, so a restart replays
    // nothing.
    let bif_before_send = compiler.compile(
        &[
            Instruction::Bif {
                op: BifOp::GcBif2,
                operands: vec![
                    Operand::Label(0),
                    Operand::Unsigned(0),
                    Operand::Unsigned(1),
                    Operand::X(0),
                    Operand::X(1),
                    Operand::X(0),
                ],
            },
            Instruction::Send,
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        2,
    );
    assert!(
        bif_before_send.is_ok(),
        "a deopt-capable Bif BEFORE the side effect must stay admitted: {bif_before_send:?}"
    );
}

/// DIAMOND MERGE-POINT TAINT (CFG-sensitive guard, union/may-reach join): an
/// effect on ONE arm of a branch, both arms joining at a merge, and a
/// deopt-capable op AFTER the merge — MUST be rejected. This is the classic
/// soundness hole: a must-join (intersection) implementation lets the effect-free
/// arm "wash" the merge and silently admit the hazard. This test alone
/// distinguishes union from must-join, and the existing same-block /
/// sequential-receive walls do not cross a merge point.
#[test]
fn diamond_merge_point_is_tainted_from_either_arm() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    // entry: select_val x0 -> {ok: arm A (Send), error: arm B (pure)}; both jump to
    // the merge M; M holds a deopt-capable RecvMarkerReserve.
    let result = compiler.compile(
        &[
            Instruction::Label { label: 1 },
            Instruction::SelectVal {
                value: Operand::X(0),
                fail: Operand::Label(5),
                list: Operand::List(vec![
                    Operand::Atom(Some(Atom::OK)),
                    Operand::Label(2),
                    Operand::Atom(Some(Atom::ERROR)),
                    Operand::Label(3),
                ]),
            },
            Instruction::Label { label: 2 }, // arm A: carries the effect
            Instruction::Send,
            Instruction::Jump {
                target: Operand::Label(4),
            },
            Instruction::Label { label: 3 }, // arm B: pure
            Instruction::Move {
                source: Operand::X(1),
                destination: Operand::X(2),
            },
            Instruction::Jump {
                target: Operand::Label(4),
            },
            Instruction::Label { label: 4 }, // merge M
            Instruction::RecvMarkerReserve {
                dest: Operand::X(3),
            },
            Instruction::Return,
            Instruction::Label { label: 5 }, // select_val fail path
            Instruction::Return,
        ],
        Atom::MODULE,
        Atom::OK,
        2,
    );
    assert!(
        matches!(result, Err(JitError::UnsupportedOpcode { .. })),
        "the merge must be tainted by the Send on arm A (union join); a must-join \
         implementation would let the pure arm B wash it and wrongly admit: {result:?}"
    );
}

/// MESSAGE-LOSS PATH (two sequential receives): `RemoveMessage` on the first
/// receive's matched path, then a second receive whose peek/wait deopt edge — a
/// deopt at the second receive restarts a function whose first receive already
/// CONSUMED a message, losing it. Must be REJECTED. (Contrast the single blocking
/// receive, which admits: there the consume is on the matched path that exits.)
#[test]
fn two_sequential_receives_with_a_consume_between_are_rejected() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let result = compiler.compile(
        &[
            Instruction::Label { label: 1 }, // receive 1
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage, // the consume (message-loss effect)
            Instruction::Jump {
                target: Operand::Label(3),
            },
            Instruction::Label { label: 2 },
            Instruction::Wait {
                fail: Operand::Label(1),
            },
            Instruction::Label { label: 3 }, // receive 2: its peek is reachable after the consume
            Instruction::LoopRec {
                fail: Operand::Label(4),
                destination: Operand::X(1),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 4 },
            Instruction::Wait {
                fail: Operand::Label(3),
            },
        ],
        Atom::MODULE,
        Atom::OK,
        0,
    );
    assert!(
        matches!(result, Err(JitError::UnsupportedOpcode { .. })),
        "a deopt at the second receive restarts a function whose first receive already \
         consumed a message (the loss path) — must be rejected: {result:?}"
    );
}

// -- JIT-002 R4: the coverage source of truth. One classification (ir_control::
// -- coverage), exhaustive with no wildcard arm, consumed by the pre-pass and
// -- dispatch catch-alls (debug-assert agreement) and by this walk.

/// One representative `Instruction` per enum variant — all 75, in enum order.
///
/// Operand shapes only matter enough to keep a Supported variant off BOTH
/// catch-alls: any operand yields at worst `UnsupportedOperand`, never
/// `UnsupportedOpcode`, EXCEPT `BinaryOp`, whose lowering is guarded by a
/// supported-sub-op check, so its representative uses a supported sub-op.
fn all_instruction_variants() -> Vec<Instruction> {
    let l = || Operand::Label(99);
    vec![
        Instruction::Label { label: 1 },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(Atom::MODULE)),
            function: Operand::Atom(Some(Atom::OK)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Move {
            source: Operand::X(1),
            destination: Operand::X(0),
        },
        Instruction::Call {
            arity: Operand::Unsigned(0),
            label: l(),
        },
        Instruction::CallOnly {
            arity: Operand::Unsigned(0),
            label: l(),
        },
        Instruction::CallExt {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::CallExtOnly {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
        },
        Instruction::Fmove {
            source: Operand::FloatRegister(0),
            dest: Operand::X(0),
        },
        Instruction::Fconv {
            source: Operand::X(0),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fadd {
            fail: l(),
            left: Operand::FloatRegister(0),
            right: Operand::FloatRegister(1),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fsub {
            fail: l(),
            left: Operand::FloatRegister(0),
            right: Operand::FloatRegister(1),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fmul {
            fail: l(),
            left: Operand::FloatRegister(0),
            right: Operand::FloatRegister(1),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fdiv {
            fail: l(),
            left: Operand::FloatRegister(0),
            right: Operand::FloatRegister(1),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fnegate {
            fail: l(),
            source: Operand::FloatRegister(0),
            dest: Operand::FloatRegister(0),
        },
        Instruction::CallLast {
            arity: Operand::Unsigned(0),
            label: l(),
            deallocate: Operand::Unsigned(0),
        },
        Instruction::CallExtLast {
            arity: Operand::Unsigned(1),
            import: Operand::Unsigned(0),
            deallocate: Operand::Unsigned(0),
        },
        Instruction::Return,
        Instruction::Allocate {
            stack_need: Operand::Unsigned(0),
            live: Operand::Unsigned(0),
        },
        Instruction::AllocateHeap {
            stack_need: Operand::Unsigned(0),
            heap_need: Operand::Unsigned(0),
            live: Operand::Unsigned(0),
        },
        Instruction::AllocateZero {
            stack_need: Operand::Unsigned(0),
            live: Operand::Unsigned(0),
        },
        Instruction::Deallocate {
            words: Operand::Unsigned(0),
        },
        Instruction::TestHeap {
            heap_need: Operand::Unsigned(0),
            live: Operand::Unsigned(0),
        },
        Instruction::PutList {
            head: Operand::X(0),
            tail: Operand::X(1),
            destination: Operand::X(0),
        },
        Instruction::PutTuple2 {
            destination: Operand::X(0),
            elements: Operand::List(vec![Operand::X(0)]),
        },
        Instruction::GetTupleElement {
            source: Operand::X(0),
            index: Operand::Unsigned(0),
            destination: Operand::X(0),
        },
        Instruction::GetList {
            source: Operand::X(0),
            head: Operand::X(0),
            tail: Operand::X(1),
        },
        Instruction::GetHd {
            source: Operand::X(0),
            destination: Operand::X(0),
        },
        Instruction::GetTl {
            source: Operand::X(0),
            destination: Operand::X(0),
        },
        Instruction::TypeTest {
            op: TypeTestOp::IsInteger,
            fail: l(),
            value: Operand::X(0),
        },
        Instruction::Comparison {
            op: ComparisonOp::EqExact,
            fail: l(),
            left: Operand::X(0),
            right: Operand::X(1),
        },
        Instruction::TestArity {
            fail: l(),
            tuple: Operand::X(0),
            arity: Operand::Unsigned(2),
        },
        Instruction::IsTaggedTuple {
            fail: l(),
            value: Operand::X(0),
            arity: Operand::Unsigned(2),
            tag: Operand::Atom(Some(Atom::OK)),
        },
        Instruction::SelectVal {
            value: Operand::X(0),
            fail: l(),
            list: Operand::List(vec![]),
        },
        Instruction::SelectTupleArity {
            value: Operand::X(0),
            fail: l(),
            list: Operand::List(vec![]),
        },
        Instruction::Jump { target: l() },
        Instruction::Bif {
            op: BifOp::Bif2,
            operands: vec![
                l(),
                Operand::Unsigned(0),
                Operand::X(0),
                Operand::X(1),
                Operand::X(0),
            ],
        },
        Instruction::Send,
        Instruction::RemoveMessage,
        Instruction::Timeout,
        Instruction::LoopRec {
            fail: l(),
            destination: Operand::X(0),
        },
        Instruction::LoopRecEnd { fail: l() },
        Instruction::Wait { fail: l() },
        Instruction::WaitTimeout {
            fail: l(),
            timeout: Operand::X(0),
        },
        Instruction::RecvMarkerReserve {
            dest: Operand::X(0),
        },
        Instruction::RecvMarkerBind {
            marker: Operand::X(0),
            reference: Operand::X(1),
        },
        Instruction::RecvMarkerClear {
            marker: Operand::X(0),
        },
        Instruction::RecvMarkerUse {
            marker: Operand::X(0),
        },
        Instruction::Catch {
            destination: Operand::Y(0),
            label: l(),
        },
        Instruction::CatchEnd {
            source: Operand::Y(0),
        },
        Instruction::Try {
            destination: Operand::Y(0),
            label: l(),
        },
        Instruction::TryEnd {
            source: Operand::Y(0),
        },
        Instruction::TryCase {
            source: Operand::Y(0),
        },
        Instruction::TryCaseEnd {
            source: Operand::Y(0),
        },
        Instruction::BinaryOp {
            op: BinaryOp::BsInitWritable,
            operands: vec![Operand::Unsigned(0), Operand::X(0)],
        },
        Instruction::MapOp {
            op: MapOp::HasMapFields,
            operands: vec![],
        },
        Instruction::MakeFun {
            operands: vec![Operand::Unsigned(0)],
        },
        Instruction::CallFun {
            arity: Operand::Unsigned(0),
        },
        Instruction::CallFun2 {
            function: Operand::X(1),
            arity: Operand::Unsigned(0),
            destination: Operand::X(0),
        },
        Instruction::Apply {
            arity: Operand::Unsigned(0),
        },
        Instruction::ApplyLast {
            arity: Operand::Unsigned(0),
            deallocate: Operand::Unsigned(0),
        },
        Instruction::Badmatch {
            value: Operand::X(0),
        },
        Instruction::Badrecord {
            value: Operand::X(0),
        },
        Instruction::CaseEnd {
            value: Operand::X(0),
        },
        Instruction::IfEnd,
        Instruction::Raise {
            stacktrace: Operand::X(0),
            reason: Operand::X(1),
        },
        Instruction::RawRaise,
        Instruction::Line {
            index: Operand::Unsigned(0),
        },
        Instruction::Trim {
            words: Operand::Unsigned(0),
            remaining: Operand::Unsigned(0),
        },
        Instruction::OnLoad,
        Instruction::BuildStacktrace,
        Instruction::Swap {
            left: Operand::X(0),
            right: Operand::X(1),
        },
        Instruction::InitYregs {
            registers: Operand::List(vec![Operand::Y(0)]),
        },
        Instruction::NifStart,
        Instruction::UpdateRecord {
            operands: vec![
                Operand::Atom(Some(Atom::OK)),
                Operand::Unsigned(1),
                Operand::X(0),
                Operand::X(0),
            ],
        },
        Instruction::Generic {
            opcode: 254,
            name: "generic",
            operands: vec![],
        },
    ]
}

/// Does compiling this slice reject it specifically as an UNSUPPORTED OPCODE
/// (the two catch-alls), as opposed to succeeding or failing on operand shape?
fn rejected_as_unsupported_opcode(
    result: &Result<crate::jit::types::NativeCode, JitError>,
) -> bool {
    matches!(result, Err(JitError::UnsupportedOpcode { .. }))
}

#[test]
fn coverage_table_has_one_entry_per_instruction_variant() {
    let variants = all_instruction_variants();
    // Exactly 75 variants, all distinct — no duplicate or missing representative.
    let distinct: std::collections::HashSet<std::mem::Discriminant<Instruction>> =
        variants.iter().map(std::mem::discriminant).collect();
    assert_eq!(
        variants.len(),
        75,
        "expected one representative per variant"
    );
    assert_eq!(
        distinct.len(),
        75,
        "representatives must be distinct variants"
    );
}

#[test]
fn coverage_walk_agrees_with_prepass_and_dispatch_for_all_75_variants() {
    let compiler = JitCompiler::new(JitSettings).unwrap();
    let variants = all_instruction_variants();

    let mut supported = 0_usize;
    for variant in &variants {
        // A slice that lets label-bearing variants resolve their target (Label 99)
        // so Supported variants reach and pass dispatch, not just the pre-pass.
        // TryEnd needs an open try scope (its lowering reports the state error as
        // an UnsupportedOpcode), so it gets a Try prelude.
        let mut slice = Vec::new();
        if matches!(variant, Instruction::TryEnd { .. }) {
            slice.push(Instruction::Try {
                destination: Operand::Y(0),
                label: Operand::Label(99),
            });
        }
        slice.push(variant.clone());
        slice.push(Instruction::Return);
        slice.push(Instruction::Label { label: 99 });
        slice.push(Instruction::Return);
        let compiled = compiler.compile(&slice, Atom::MODULE, Atom::OK, 0);
        let is_opcode_rejected = rejected_as_unsupported_opcode(&compiled);

        match coverage(variant) {
            Coverage::Supported => {
                supported += 1;
                // (a) + (c): a Supported variant hits neither the pre-pass nor the
                // dispatch UnsupportedOpcode catch-all.
                assert!(
                    !is_opcode_rejected,
                    "coverage table marks {variant:?} Supported but compilation \
                     rejected it as an unsupported opcode: {compiled:?}",
                );
            }
            Coverage::RejectedIncremental { .. } | Coverage::RejectedInherent { .. } => {
                // (b): a Rejected variant is rejected as an unsupported opcode.
                assert!(
                    is_opcode_rejected,
                    "coverage table marks {variant:?} Rejected but compilation did \
                     not reject it as an unsupported opcode: {compiled:?}",
                );
            }
        }
    }

    // The Supported count is DERIVED from the table over the walk, not a
    // duplicated literal. Post-R1/R2/R3: 47 baseline + 12 (R1 8 + R2 3 + R3 1).
    assert_eq!(
        supported, 59,
        "Supported count derived from the coverage table"
    );
}
