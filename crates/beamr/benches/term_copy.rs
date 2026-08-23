//! term_copy — price the heap→Vec→heap copies the accessor-lifetimes landing
//! added on the mailbox-send, ETS and binary-match paths (beamr#35).
//!
//! DIFFERENTIAL BENCH. This file must compile UNCHANGED at both arms — the
//! pre-landing base (`c55ac360`) and land — so it drives BEAM-level operations
//! and arm-stable public API only, and NEVER the changed accessors
//! (`as_bytes()`/`limbs()` take a `HeapBorrow` at land only). The differential
//! runner copies this file plus its `[[bench]]` stanza into the base worktree.
//!
//! Design of record: beamr#35, the filer's amendment comment. The arms:
//!
//! 1. `mailbox_send` — `MailboxSender::send` copies a boxed term into the
//!    receiver heap; land materialises an owned Vec first (`copy_binary`,
//!    `copy_sub_binary`, `copy_bigint`).
//! 2. `ets_insert` — `copy_term_to_ets`: expected ~flat except bigint (the
//!    amendment's corrected direction).
//! 3. `ets_readback` — `copy_term_to_heap`: four new allocations at land.
//! 4. `bs_get_binary` — the hot one, per opcode: `BsGetBinary2` + `BsGetTail`.
//! 5. null arms — `BsGetInteger2` / `BsMatchString` took the identical
//!    signature churn with ZERO new allocation. If these move between arms,
//!    the instrument is measuring churn or venue noise, not the copy, and no
//!    other arm's number may be trusted.
//! 6. control — a deliberate `Vec` allocation per size class, so a flat
//!    result cannot be read as "no cost" by a harness that could never have
//!    resolved one.
//!
//! Pre-registered falsifier (kept verbatim from the design record): for the
//! binary-copy arms the added cost must SCALE WITH PAYLOAD SIZE; if arm 4's
//! delta is flat in payload size, #35 is re-opened as mis-diagnosed — not
//! re-fitted.
//!
//! Topology note: interleaving A/B/A/B happens at the RUNNER level (alternate
//! whole invocations between arm checkouts); a single invocation of this
//! binary measures one arm.

use beamr::ets::{copy_term_to_ets, copy_term_to_heap};
use beamr::interpreter::opcodes::binary::binary_op;
use beamr::loader::decode::compact::Operand;
use beamr::loader::{Instruction, Literal};
use beamr::module::{Module, ModuleOrigin};
use beamr::process::Process;
use beamr::process::heap::Heap;
use beamr::term::Term;
use beamr::term::binary::{Binary, packed_word_count, write_binary};
use beamr::term::boxed::write_bigint;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::collections::HashMap;
use std::hint::black_box;

// BinaryOp is re-exported at `loader::decode` (`decode::instruction` itself
// is private).
use beamr::loader::decode::BinaryOp;

/// Payload sizes in bytes. Straddles `REFC_BINARY_THRESHOLD` (64): the copy
/// path under test (`copy_binary` → owned bytes → `write_binary`) is the same
/// at every size, so the axis exists to expose payload scaling, which is the
/// falsifier's load-bearing prediction.
const BINARY_SIZES: &[usize] = &[16, 64, 1024, 65536];

/// Bigint widths in limbs (8 bytes each).
const BIGINT_LIMBS: &[usize] = &[1, 8, 64, 512];

const FAIL_LABEL: u32 = 9;

fn heap_binary(heap: &mut Heap, bytes: &[u8]) -> Term {
    let words = 2 + packed_word_count(bytes.len());
    let slice = heap.alloc_slice(words).expect("bench heap fits binary");
    write_binary(slice, bytes).expect("bench binary fits")
}

fn heap_bigint(heap: &mut Heap, limbs: &[u64]) -> Term {
    let words = 3 + limbs.len();
    let slice = heap.alloc_slice(words).expect("bench heap fits bigint");
    write_bigint(slice, false, limbs).expect("bench bigint fits")
}

/// Words needed to hold one copy of a `size`-byte binary, with slack for the
/// mailbox's message bookkeeping.
fn binary_heap_words(size: usize) -> usize {
    2 * (2 + packed_word_count(size)) + 64
}

fn bigint_heap_words(limbs: usize) -> usize {
    2 * (3 + limbs) + 64
}

fn bare_module(code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name: beamr::atom::Atom::OK,
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

/// Arm 1 — mailbox send. Fresh receiver heap (and mailbox) per iteration so
/// the measured copy always lands in young, uncontended space; the source
/// term is built once and outlives the run.
fn bench_mailbox_send(c: &mut Criterion) {
    let mut group = c.benchmark_group("mailbox_send");
    for &size in BINARY_SIZES {
        let mut source_heap = Heap::new(binary_heap_words(size));
        let payload = vec![0xa5u8; size];
        let term = heap_binary(&mut source_heap, &payload);
        group.bench_function(format!("binary/{size}"), |b| {
            b.iter_batched(
                || {
                    (
                        beamr::mailbox::Mailbox::new(),
                        Heap::new(binary_heap_words(size)),
                    )
                },
                |(mailbox, mut receiver_heap)| {
                    mailbox
                        .sender()
                        .send(black_box(term), &mut receiver_heap)
                        .expect("send fits");
                    black_box((mailbox, receiver_heap));
                },
                BatchSize::SmallInput,
            );
        });
    }
    for &limbs in BIGINT_LIMBS {
        let mut source_heap = Heap::new(bigint_heap_words(limbs));
        let limb_data = vec![0x5au64; limbs];
        let term = heap_bigint(&mut source_heap, &limb_data);
        group.bench_function(format!("bigint/{limbs}"), |b| {
            b.iter_batched(
                || {
                    (
                        beamr::mailbox::Mailbox::new(),
                        Heap::new(bigint_heap_words(limbs)),
                    )
                },
                |(mailbox, mut receiver_heap)| {
                    mailbox
                        .sender()
                        .send(black_box(term), &mut receiver_heap)
                        .expect("send fits");
                    black_box((mailbox, receiver_heap));
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Arm 2 — ETS insert. The amendment's corrected direction says this arm is
/// ~flat except bigint; it exists so that prediction is measured rather than
/// assumed.
fn bench_ets_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("ets_insert");
    for &size in BINARY_SIZES {
        let mut source_heap = Heap::new(binary_heap_words(size));
        let payload = vec![0xa5u8; size];
        let term = heap_binary(&mut source_heap, &payload);
        group.bench_function(format!("binary/{size}"), |b| {
            b.iter(|| black_box(copy_term_to_ets(black_box(term)).expect("ets copy")));
        });
    }
    for &limbs in BIGINT_LIMBS {
        let mut source_heap = Heap::new(bigint_heap_words(limbs));
        let limb_data = vec![0x5au64; limbs];
        let term = heap_bigint(&mut source_heap, &limb_data);
        group.bench_function(format!("bigint/{limbs}"), |b| {
            b.iter(|| black_box(copy_term_to_ets(black_box(term)).expect("ets copy")));
        });
    }
    group.finish();
}

/// Arm 3 — ETS read-back, where four of the five ETS-side allocations landed.
fn bench_ets_readback(c: &mut Criterion) {
    let mut group = c.benchmark_group("ets_readback");
    for &size in BINARY_SIZES {
        let mut source_heap = Heap::new(binary_heap_words(size));
        let payload = vec![0xa5u8; size];
        let term = heap_binary(&mut source_heap, &payload);
        let owned = copy_term_to_ets(term).expect("ets copy");
        group.bench_function(format!("binary/{size}"), |b| {
            b.iter_batched(
                || Heap::new(binary_heap_words(size)),
                |mut heap| {
                    let copied =
                        copy_term_to_heap(black_box(owned.root()), &mut heap).expect("readback");
                    black_box((copied, heap));
                },
                BatchSize::SmallInput,
            );
        });
    }
    for &limbs in BIGINT_LIMBS {
        let mut source_heap = Heap::new(bigint_heap_words(limbs));
        let limb_data = vec![0x5au64; limbs];
        let term = heap_bigint(&mut source_heap, &limb_data);
        let owned = copy_term_to_ets(term).expect("ets copy");
        group.bench_function(format!("bigint/{limbs}"), |b| {
            b.iter_batched(
                || Heap::new(bigint_heap_words(limbs)),
                |mut heap| {
                    let copied =
                        copy_term_to_heap(black_box(owned.root()), &mut heap).expect("readback");
                    black_box((copied, heap));
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Fresh process holding a `size`-byte source binary with a started match
/// context in x1. The module carries only the fail label.
fn match_process(size: usize) -> (Process, Module) {
    let module = bare_module(vec![Instruction::Label { label: FAIL_LABEL }]);
    // Heap: source binary + match context + one extracted sub-binary + slack.
    let mut process = Process::new(1, 4 * (2 + packed_word_count(size)) + 128);
    let payload = vec![0xa5u8; size];
    let source = heap_binary(process.heap_mut(), &payload);
    process.set_x_reg(0, source);
    binary_op(
        &mut process,
        &module,
        BinaryOp::BsStartMatch3,
        &[Operand::Label(FAIL_LABEL), Operand::X(0), Operand::X(1)],
    )
    .expect("start match");
    (process, module)
}

/// Arm 4 — the hot one. One `BsGetBinary2` taking all but the last 8 bytes,
/// then `BsGetTail` for the rest: two of the four changed match-path sites,
/// exercised once per iteration so the per-opcode cost is what criterion sees.
fn bench_bs_get_binary(c: &mut Criterion) {
    let mut group = c.benchmark_group("bs_get_binary");
    for &size in BINARY_SIZES {
        let head_bits = ((size - 8) * 8) as u64;
        group.bench_function(format!("binary/{size}"), |b| {
            b.iter_batched(
                || match_process(size),
                |(mut process, module)| {
                    binary_op(
                        &mut process,
                        &module,
                        BinaryOp::BsGetBinary2,
                        &[
                            Operand::Label(FAIL_LABEL),
                            Operand::X(1),
                            Operand::Unsigned(head_bits),
                            Operand::Unsigned(1),
                            Operand::Atom(None),
                            Operand::X(2),
                        ],
                    )
                    .expect("get binary");
                    binary_op(
                        &mut process,
                        &module,
                        BinaryOp::BsGetTail,
                        &[Operand::X(1), Operand::X(3), Operand::Unsigned(0)],
                    )
                    .expect("get tail");
                    // Sanity via the arm-stable length accessor only — never
                    // the bytes.
                    debug_assert_eq!(
                        Binary::new(process.x_reg(2)).map(Binary::len),
                        Some(size - 8)
                    );
                    black_box(process);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Arm 5a — null: `BsGetInteger2` took the same `HeapBorrow` rewrite with no
/// new allocation. 64 one-byte reads over a 64-byte source per iteration.
fn bench_null_bs_get_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("null_bs_get_integer");
    group.bench_function("binary/64", |b| {
        b.iter_batched(
            || match_process(64),
            |(mut process, module)| {
                for _ in 0..64 {
                    binary_op(
                        &mut process,
                        &module,
                        BinaryOp::BsGetInteger2,
                        &[
                            Operand::Label(FAIL_LABEL),
                            Operand::X(1),
                            Operand::Unsigned(8),
                            Operand::Unsigned(1),
                            Operand::Atom(None),
                            Operand::X(2),
                        ],
                    )
                    .expect("get integer");
                }
                debug_assert_eq!(process.x_reg(2).as_small_int(), Some(0xa5));
                black_box(process);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

/// Arm 5b — null: `BsMatchString` matches the whole 64-byte source against a
/// module literal, one opcode per iteration. Allocates nothing on either arm.
fn bench_null_bs_match_string(c: &mut Criterion) {
    let mut group = c.benchmark_group("null_bs_match_string");
    let payload = vec![0xa5u8; 64];
    let literals = vec![Literal::String(payload)];
    let constant_pool =
        beamr::constant_pool::materialise_literals(&literals, None).expect("literal pool");
    group.bench_function("binary/64", |b| {
        b.iter_batched(
            || {
                let (process, mut module) = match_process(64);
                module.literals = literals.clone();
                module.constant_pool = constant_pool.clone();
                (process, module)
            },
            |(mut process, module)| {
                binary_op(
                    &mut process,
                    &module,
                    BinaryOp::BsMatchString,
                    &[
                        Operand::Label(FAIL_LABEL),
                        Operand::X(1),
                        Operand::Unsigned(64 * 8),
                        Operand::Literal(0),
                    ],
                )
                .expect("match string");
                black_box(process);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

/// Arm 6 — positive control: one deliberate allocate-and-memcpy per size
/// class. If an arm above reads flat while this arm resolves cleanly at the
/// same size, the flatness is a finding; if THIS arm is lost in noise, the
/// harness cannot price the copy and no flat reading elsewhere means anything.
fn bench_control_vec_alloc(c: &mut Criterion) {
    let mut group = c.benchmark_group("control_vec_alloc");
    for &size in BINARY_SIZES {
        let payload = vec![0xa5u8; size];
        group.bench_function(format!("binary/{size}"), |b| {
            b.iter(|| black_box(black_box(payload.as_slice()).to_vec()));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_mailbox_send,
    bench_ets_insert,
    bench_ets_readback,
    bench_bs_get_binary,
    bench_null_bs_get_integer,
    bench_null_bs_match_string,
    bench_control_vec_alloc,
);
criterion_main!(benches);
