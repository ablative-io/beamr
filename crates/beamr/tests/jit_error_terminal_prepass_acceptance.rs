//! BEAMR-R8-DEOPT R5 (beamr-side arm) — the PRE-PASS ACCEPTANCE census.
//!
//! R5's beamr-side falsifier: specimen functions containing case-fallthrough and
//! assert edges COMPILE — the pre-pass accepts them — and their happy paths run
//! native. The happy paths running native is witnessed by the R3/R3b fixtures
//! (`jit_error_terminal_identity.rs`, `jit_badmatch_restart_fidelity.rs`), which
//! assert a live `JitCache` entry before driving. What this file adds is the
//! ADMISSION census itself, reported PER FUNCTION WITH ITS REASON — never a bare
//! count — so a rejection that is CORRECT is separable from an incomplete fix.
//!
//! Three rejection classes are deliberately included and named, because a census
//! that only shows acceptances cannot be read:
//!
//!   * F6 — a trap reachable AFTER an observable side effect is still rejected,
//!     by `reject_deopt_after_side_effect`, and that rejection is CORRECT: a
//!     deopt restarts the callee from its bytecode entry, so the effect would
//!     replay. This is the same guard that protects `func_info` and the
//!     recv-markers. At the estate-side instrument it measures as "not
//!     0/9,864" and would read as an incomplete fix when it is not.
//!   * The AWL `let assert` LIST burst — `ablative-io/aion`
//!     `crates/aion-awl/src/mir/select/emit/burst.rs:344-396` — walks with
//!     `TypeTestOp::IsNonemptyList` and terminates on `IsNil`. This tier lowers
//!     NEITHER (`jit/ir_control_validation.rs`), so a function carrying that
//!     burst is STILL rejected after R8 — for a reason that is not the terminal.
//!     Recorded as a finding; it bears directly on the estate-side R5 arm.
//!   * The wave-2 exception machinery — `Catch` / `CatchEnd` / `TryCaseEnd` /
//!     `Raise` / `RawRaise` / `BuildStacktrace` — stays rejected, untouched.
//!     That is the Wall, asserted rather than assumed.
//!
//! First-party fixture note: beamr is our own product, this box is our own fleet
//! venue, and `ablative-io/aion` is our own public repo; every corpus document
//! and specimen here is a first-party local test fixture.

use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::jit::{JitCompiler, JitSettings};
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::loader::decode::{ComparisonOp, TypeTestOp};
use beamr::loader::load_module;
use beamr::module::ModuleRegistry;
use beamr::native::BifRegistryImpl;

/// A specimen's admission verdict, with the reason when it is refused.
#[derive(Debug, Eq, PartialEq)]
enum Verdict {
    Accepted,
    Rejected(String),
}

impl Verdict {
    fn of(compiler: &JitCompiler, code: &[Instruction], atoms: &AtomTable, name: &str) -> Self {
        let module = atoms.intern("prepass_census");
        let function = atoms.intern(name);
        match compiler.compile(code, module, function, 1) {
            Ok(_) => Verdict::Accepted,
            Err(error) => Verdict::Rejected(format!("{error}")),
        }
    }

    fn is_accepted(&self) -> bool {
        matches!(self, Verdict::Accepted)
    }

    fn reason(&self) -> &str {
        match self {
            Verdict::Accepted => "-",
            Verdict::Rejected(reason) => reason,
        }
    }
}

/// A minimal admissible function body whose cold edge reaches `terminal`.
/// `prefix` is spliced in before the guard, which is how the side-effect arm
/// puts an observable effect on the path to the trap.
fn specimen(
    prefix: Vec<Instruction>,
    terminal: Instruction,
    atoms: &AtomTable,
) -> Vec<Instruction> {
    let pass = Operand::Atom(Some(atoms.intern("pass")));
    let mut code = vec![Instruction::Label { label: 1 }];
    code.extend(prefix);
    code.extend([
        Instruction::Comparison {
            op: ComparisonOp::EqExact,
            fail: Operand::Label(2),
            left: Operand::X(0),
            right: pass,
        },
        Instruction::Return,
        Instruction::Label { label: 2 },
        terminal,
    ]);
    code
}

fn terminals(atoms: &AtomTable) -> Vec<(&'static str, Instruction)> {
    let _ = atoms;
    vec![
        (
            "Badmatch",
            Instruction::Badmatch {
                value: Operand::X(0),
            },
        ),
        (
            "Badrecord",
            Instruction::Badrecord {
                value: Operand::X(0),
            },
        ),
        (
            "CaseEnd",
            Instruction::CaseEnd {
                value: Operand::X(0),
            },
        ),
        ("IfEnd", Instruction::IfEnd),
    ]
}

/// The wave-2 exception machinery, which this brief's Wall leaves untouched.
fn wave_two(atoms: &AtomTable) -> Vec<(&'static str, Instruction)> {
    vec![
        (
            "Catch",
            Instruction::Catch {
                destination: Operand::Y(0),
                label: Operand::Label(1),
            },
        ),
        (
            "CatchEnd",
            Instruction::CatchEnd {
                source: Operand::Y(0),
            },
        ),
        (
            "TryCaseEnd",
            Instruction::TryCaseEnd {
                source: Operand::X(0),
            },
        ),
        (
            "Raise",
            Instruction::Raise {
                stacktrace: Operand::X(0),
                reason: Operand::Atom(Some(atoms.intern("boom"))),
            },
        ),
        ("RawRaise", Instruction::RawRaise),
        ("BuildStacktrace", Instruction::BuildStacktrace),
    ]
}

#[test]
fn the_four_error_terminals_are_accepted_by_the_prepass_and_lower() {
    let atoms = AtomTable::with_common_atoms();
    let compiler = JitCompiler::new(JitSettings).expect("jit compiler initializes");

    println!("---- BEAMR-R8-DEOPT R5: pre-pass acceptance, per specimen ----");
    let mut accepted = 0;
    for (name, terminal) in terminals(&atoms) {
        let code = specimen(Vec::new(), terminal, &atoms);
        let verdict = Verdict::of(&compiler, &code, &atoms, name);
        println!(
            "  cold-edge specimen {name:<10} : {:<8} reason: {}",
            if verdict.is_accepted() {
                "ACCEPTED"
            } else {
                "REJECTED"
            },
            verdict.reason()
        );
        assert!(
            verdict.is_accepted(),
            "{name}: a function whose only cold edge is this terminal must be \
             ADMITTED — beamr's pre-pass rejection is whole-function, so before \
             R8 this rejected the entire containing function. Reason given: {}",
            verdict.reason()
        );
        accepted += 1;
    }
    assert_eq!(accepted, 4, "all four terminals are in this census");
}

/// F6. A trap reachable AFTER an observable side effect is still refused, and
/// the refusal names itself. This is a CORRECT rejection, not an incomplete fix,
/// and the estate-side R5 arm must be able to separate the two populations.
#[test]
fn a_terminal_after_an_observable_side_effect_is_still_refused_and_names_why() {
    let atoms = AtomTable::with_common_atoms();
    let compiler = JitCompiler::new(JitSettings).expect("jit compiler initializes");

    println!("---- BEAMR-R8-DEOPT R5 / F6: traps after an observable side effect ----");
    for (name, terminal) in terminals(&atoms) {
        let code = specimen(vec![Instruction::Send], terminal, &atoms);
        let verdict = Verdict::of(&compiler, &code, &atoms, name);
        println!(
            "  Send then {name:<10} : {:<8} reason: {}",
            if verdict.is_accepted() {
                "ACCEPTED"
            } else {
                "REJECTED"
            },
            verdict.reason()
        );
        assert!(
            !verdict.is_accepted(),
            "{name} after a Send must be REFUSED: a deopt restarts the callee \
             from its bytecode entry, so the Send would be replayed"
        );
        assert!(
            verdict
                .reason()
                .contains("reachable after an observable side effect"),
            "{name}: the refusal must name the deopt-after-side-effect guard, so \
             a CORRECT rejection is separable from a classification failure at \
             the estate-side instrument. Got: {}",
            verdict.reason()
        );
    }
}

/// The Wall, asserted. The exception machinery stays wave-2 and stays refused.
#[test]
fn the_wave_two_exception_machinery_is_untouched_and_still_refused() {
    let atoms = AtomTable::with_common_atoms();
    let compiler = JitCompiler::new(JitSettings).expect("jit compiler initializes");

    println!("---- BEAMR-R8-DEOPT R5: the scope wall, asserted ----");
    for (name, instruction) in wave_two(&atoms) {
        let code = specimen(Vec::new(), instruction, &atoms);
        let verdict = Verdict::of(&compiler, &code, &atoms, name);
        println!(
            "  wave-2 {name:<16} : {:<8} reason: {}",
            if verdict.is_accepted() {
                "ACCEPTED"
            } else {
                "REJECTED"
            },
            verdict.reason()
        );
        assert!(
            !verdict.is_accepted(),
            "{name} is wave-2 and outside this brief's scope; admitting it here \
             would be scope creep the Wall forbids"
        );
    }
}

/// The AWL `let assert` LIST burst, verbatim in shape from `ablative-io/aion`
/// `crates/aion-awl/src/mir/select/emit/burst.rs:344-396`. It is STILL refused
/// after R8, and the reason is NOT the terminal — it is the two type tests this
/// tier does not lower. This bears directly on the estate-side R5 arm: the
/// 51/9,864 poisoned set does not clear on the terminals alone.
#[test]
fn the_awl_list_burst_is_still_refused_but_not_for_its_terminal() {
    let atoms = AtomTable::with_common_atoms();
    let compiler = JitCompiler::new(JitSettings).expect("jit compiler initializes");

    // burst.rs:349-395, one bind unrolled, with the BC-2b-5 subject Move.
    let burst = vec![
        Instruction::Label { label: 1 },
        Instruction::Move {
            source: Operand::X(0),
            destination: Operand::Y(0),
        },
        Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        },
        Instruction::TypeTest {
            op: TypeTestOp::IsNonemptyList,
            fail: Operand::Label(2),
            value: Operand::X(0),
        },
        Instruction::GetList {
            source: Operand::X(0),
            head: Operand::X(1),
            tail: Operand::X(2),
        },
        Instruction::Move {
            source: Operand::X(2),
            destination: Operand::X(0),
        },
        Instruction::TypeTest {
            op: TypeTestOp::IsNil,
            fail: Operand::Label(2),
            value: Operand::X(0),
        },
        Instruction::Jump {
            target: Operand::Label(3),
        },
        Instruction::Label { label: 2 },
        Instruction::Move {
            source: Operand::Y(0),
            destination: Operand::X(0),
        },
        Instruction::Badmatch {
            value: Operand::X(0),
        },
        Instruction::Label { label: 3 },
        Instruction::Return,
    ];
    let verdict = Verdict::of(&compiler, &burst, &atoms, "awl_assert_list");
    println!("---- BEAMR-R8-DEOPT R5: the AWL let-assert LIST burst ----");
    println!(
        "  awl assert_list burst : {:<8} reason: {}",
        if verdict.is_accepted() {
            "ACCEPTED"
        } else {
            "REJECTED"
        },
        verdict.reason()
    );
    assert!(
        !verdict.is_accepted(),
        "measured: this shape is refused today. If it ever becomes accepted, \
         update this test and the finding it carries"
    );
    assert!(
        verdict.reason().contains("IsNonemptyList") || verdict.reason().contains("IsNil"),
        "the refusal must be about the TYPE TEST, not the terminal — that is the \
         whole point of this specimen. Got: {}",
        verdict.reason()
    );
    assert!(
        !verdict.reason().to_lowercase().contains("badmatch"),
        "the refusal must NOT be about Badmatch — R8 lowered it. Got: {}",
        verdict.reason()
    );
}

/// The corpus-derived fixture's own functions, function by function, with the
/// reason for every refusal. This is the census the flight report carries.
#[test]
fn the_corpus_derived_fixture_admission_census_is_reported_per_function() {
    let atoms = AtomTable::with_common_atoms();
    let bifs = BifRegistryImpl::new();
    let registry = Arc::new(ModuleRegistry::new());
    load_module(
        include_bytes!("fixtures/awl_terminals.beam"),
        &atoms,
        &registry,
        &bifs,
    )
    .expect("awl_terminals fixture loads");
    let module = registry
        .lookup(atoms.intern("awl_terminals"))
        .expect("fixture module is registered");
    let compiler = JitCompiler::new(JitSettings).expect("jit compiler initializes");

    // The four terminal-carrying specimens, plus the erlc list destructure that
    // reaches Badmatch through the type tests this tier does not lower.
    let specimens = [
        ("verdict_code", "CaseEnd", true),
        ("report_presence", "IfEnd", true),
        ("gate_passed", "Badrecord", true),
        (
            "first_failing_target",
            "Badmatch (via is_nonempty_list/is_nil)",
            false,
        ),
    ];

    println!("---- BEAMR-R8-DEOPT R5: corpus-derived specimen census ----");
    for (name, terminal, expect_accepted) in specimens {
        let function = atoms.intern(name);
        let entry = module.export_ip(function, 1).expect("specimen is exported");
        let code = module
            .function_instructions(entry)
            .expect("specimen has a body");
        let verdict = Verdict::of(&compiler, code, &atoms, name);
        println!(
            "  {name:<22} [{terminal:<38}] : {:<8} reason: {}",
            if verdict.is_accepted() {
                "ACCEPTED"
            } else {
                "REJECTED"
            },
            verdict.reason()
        );
        assert_eq!(
            verdict.is_accepted(),
            expect_accepted,
            "{name}: admission changed. Reason given: {}",
            verdict.reason()
        );
        if !expect_accepted {
            assert!(
                !verdict.reason().to_lowercase().contains("badmatch"),
                "{name} is refused for its TYPE TESTS, not for its terminal — a \
                 refusal naming Badmatch would mean R8 did not land. Got: {}",
                verdict.reason()
            );
        }
    }
}
