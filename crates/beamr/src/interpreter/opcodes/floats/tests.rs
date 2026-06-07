use super::*;
use crate::atom::Atom;
use crate::interpreter::opcodes::dispatch;
use crate::loader::Instruction;
use crate::module::Module;
use std::collections::HashMap;

fn module(code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name: Atom::OK,
        generation: 0,
        exports: HashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    }
}

fn boxed_float(process: &mut Process, value: f64) -> Term {
    let ptr = process.heap_mut().alloc(2).expect("test heap has room");
    let heap = core::heap_slice(ptr, 2);
    write_float(heap, value).expect("float layout fits")
}

fn x_float(process: &Process, index: u16) -> f64 {
    Float::new(process.x_reg(index))
        .expect("boxed float")
        .value()
}

#[test]
fn fmove_moves_boxed_float_into_float_register() {
    let mut process = Process::new(1, 16);
    let module = module(vec![]);
    let term = boxed_float(&mut process, 3.14);
    process.set_x_reg(0, term);

    assert_eq!(
        fmove(
            &mut process,
            &module,
            &Operand::X(0),
            &Operand::FloatRegister(0),
        ),
        Ok(InstructionOutcome::Continue)
    );
    assert_eq!(process.get_float_reg(0), Ok(3.14));
}

#[test]
fn fmove_boxes_float_register_into_x_register() {
    let mut process = Process::new(1, 16);
    let module = module(vec![]);
    process.set_float_reg(0, 3.14).expect("set fr0");

    assert_eq!(
        fmove(
            &mut process,
            &module,
            &Operand::FloatRegister(0),
            &Operand::X(0),
        ),
        Ok(InstructionOutcome::Continue)
    );
    assert_eq!(x_float(&process, 0), 3.14);
}

#[test]
fn fmove_copies_between_float_registers() {
    let mut process = Process::new(1, 16);
    let module = module(vec![]);
    process.set_float_reg(0, -7.25).expect("set fr0");

    fmove(
        &mut process,
        &module,
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(1),
    )
    .expect("fmove");

    assert_eq!(process.get_float_reg(1), Ok(-7.25));
}

#[test]
fn fconv_converts_integer_and_boxed_float_sources() {
    let mut process = Process::new(1, 16);
    let module = module(vec![]);

    fconv(
        &mut process,
        &module,
        &Operand::Integer(42),
        &Operand::FloatRegister(0),
    )
    .expect("integer fconv");
    assert_eq!(process.get_float_reg(0), Ok(42.0));

    let term = boxed_float(&mut process, -1.5);
    process.set_x_reg(0, term);
    fconv(
        &mut process,
        &module,
        &Operand::X(0),
        &Operand::FloatRegister(1),
    )
    .expect("float fconv");
    assert_eq!(process.get_float_reg(1), Ok(-1.5));
}

#[test]
fn float_arithmetic_ops_write_dest_registers() {
    let mut process = Process::new(1, 16);
    process.set_float_reg(0, 1.5).expect("fr0");
    process.set_float_reg(1, 2.5).expect("fr1");

    fadd(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(1),
        &Operand::FloatRegister(2),
    )
    .expect("fadd");
    fsub(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(1),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(3),
    )
    .expect("fsub");
    fmul(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(1),
        &Operand::FloatRegister(4),
    )
    .expect("fmul");

    assert_eq!(process.get_float_reg(2), Ok(4.0));
    assert_eq!(process.get_float_reg(3), Ok(1.0));
    assert_eq!(process.get_float_reg(4), Ok(3.75));
}

#[test]
fn fdiv_divides_or_returns_badarith_for_zero_denominator() {
    let mut process = Process::new(1, 16);
    process.set_float_reg(0, 10.0).expect("fr0");
    process.set_float_reg(1, 2.0).expect("fr1");

    fdiv(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(1),
        &Operand::FloatRegister(2),
    )
    .expect("fdiv");
    assert_eq!(process.get_float_reg(2), Ok(5.0));

    process.set_float_reg(1, 0.0).expect("fr1 zero");
    assert_eq!(
        fdiv(
            &mut process,
            &Operand::Label(0),
            &Operand::FloatRegister(0),
            &Operand::FloatRegister(1),
            &Operand::FloatRegister(2),
        ),
        Err(ExecError::Badarith)
    );
}

#[test]
fn fnegate_negates_and_preserves_nan_and_infinity_edges() {
    let mut process = Process::new(1, 16);
    process.set_float_reg(0, 3.14).expect("fr0");
    process.set_float_reg(2, f64::NAN).expect("fr2");
    process.set_float_reg(3, f64::INFINITY).expect("fr3");

    fnegate(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(1),
    )
    .expect("fnegate normal");
    fadd(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(2),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(4),
    )
    .expect("fadd nan");
    fmul(
        &mut process,
        &Operand::Label(0),
        &Operand::FloatRegister(3),
        &Operand::FloatRegister(0),
        &Operand::FloatRegister(5),
    )
    .expect("fmul inf");

    assert_eq!(process.get_float_reg(1), Ok(-3.14));
    assert!(process.get_float_reg(4).expect("fr4").is_nan());
    assert_eq!(process.get_float_reg(5), Ok(f64::INFINITY));
}

#[test]
fn dispatch_executes_compiled_float_instruction_sequence() {
    let mut process = Process::new(1, 16);
    let code = vec![
        Instruction::Fconv {
            source: Operand::X(0),
            dest: Operand::FloatRegister(0),
        },
        Instruction::Fmove {
            source: Operand::X(1),
            dest: Operand::FloatRegister(1),
        },
        Instruction::Fadd {
            fail: Operand::Label(0),
            left: Operand::FloatRegister(0),
            right: Operand::FloatRegister(1),
            dest: Operand::FloatRegister(2),
        },
        Instruction::Fmove {
            source: Operand::FloatRegister(2),
            dest: Operand::X(2),
        },
    ];
    let module = module(code.clone());
    let one = boxed_float(&mut process, 1.0);
    process.set_x_reg(0, Term::small_int(41));
    process.set_x_reg(1, one);

    for (ip, instruction) in code.iter().enumerate() {
        assert_eq!(
            dispatch(&mut process, &module, instruction, ip + 1, None),
            Ok(InstructionOutcome::Continue)
        );
    }

    assert_eq!(x_float(&process, 2), 42.0);
}

#[test]
fn fmove_float_register_to_x_is_gc_safe() {
    let mut process = Process::new(1, 2);
    let module = module(vec![]);
    let root = boxed_float(&mut process, 7.0);
    process.set_x_reg(0, root);
    process.set_float_reg(0, 3.5).expect("set fr0");

    fmove(
        &mut process,
        &module,
        &Operand::FloatRegister(0),
        &Operand::X(1),
    )
    .expect("fmove with GC");

    assert_eq!(x_float(&process, 0), 7.0);
    assert_eq!(x_float(&process, 1), 3.5);
}
