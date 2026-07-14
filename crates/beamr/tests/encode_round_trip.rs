//! The round-trip ratchet for the `.beam` encoder (feature `encode`).
//!
//! For every committed `.beam` fixture: `decode(x) -> encode -> decode` and
//! assert the two `ParsedModule` contents are equal. Where the fixture's
//! imports resolve, the re-encoded module is also run through
//! `validate_module`, held to the same verdict the original earns.
#![cfg(feature = "encode")]

use std::path::{Path, PathBuf};

use beamr::atom::AtomTable;
use beamr::loader::decode::{Instruction, Literal, Operand, TypeTestOp};
use beamr::loader::encode::encode_module;
use beamr::loader::load::{ParsedModule, resolve_imports};
use beamr::loader::validate::validate_module;
use beamr::loader::{ExportEntry, load_beam_chunks};
use beamr::module::ModuleRegistry;
use beamr::native::{AllCapabilitiesPolicy, BifRegistry, NativeEntry};

struct NoBifs;

impl BifRegistry for NoBifs {
    fn lookup(
        &self,
        _module: beamr::atom::Atom,
        _function: beamr::atom::Atom,
        _arity: u8,
    ) -> Option<NativeEntry> {
        None
    }
}

/// Recursively collects every `*.beam` file under `root`.
fn collect_beams(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_beams(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "beam") {
            out.push(path);
        }
    }
}

fn all_fixtures() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut roots = vec![manifest.join("tests/fixtures")];
    // `test-workflows/` sits at the repository root, two levels above the crate.
    if let Some(repo_root) = manifest.parent().and_then(Path::parent) {
        roots.push(repo_root.join("test-workflows"));
    }
    let mut beams = Vec::new();
    for root in roots {
        collect_beams(&root, &mut beams);
    }
    beams.sort();
    beams
}

/// The result of driving one `.beam` file through the encoder.
enum Outcome {
    /// `decode -> encode -> decode` produced a structurally identical module.
    Passed,
    /// The original bytes could not even be decoded — the decoder, not the
    /// encoder, declined them (e.g. an opcode outside beamr's supported set).
    /// The encoder is never exercised, so this is an exclusion, not a bug.
    Excluded(String),
    /// The encoder was exercised and misbehaved: it errored, produced bytes the
    /// loader rejects, or produced a module that differs from the original.
    /// Every one of these is an encoder bug to fix.
    Failed(String),
}

/// Drives one `.beam` payload through `decode -> encode -> decode` and reports
/// whether the encoder round-tripped it, was never reached (undecodable input),
/// or misbehaved.
fn drive(bytes: &[u8]) -> Outcome {
    let table = AtomTable::with_common_atoms();
    let original = match load_beam_chunks(bytes, &table) {
        Ok(module) => module,
        Err(error) => return Outcome::Excluded(format!("original does not decode: {error}")),
    };
    let encoded = match encode_module(&original, &table) {
        Ok(bytes) => bytes,
        Err(error) => return Outcome::Failed(format!("encode failed: {error}")),
    };
    let reloaded = match load_beam_chunks(&encoded, &table) {
        Ok(module) => module,
        Err(error) => return Outcome::Failed(format!("re-encoded bytes do not decode: {error}")),
    };
    if let Some(detail) = first_mismatch(&original, &reloaded) {
        return Outcome::Failed(format!("round-trip mismatch: {detail}"));
    }
    // The re-encoded module must earn the same validation verdict as the
    // original — encoding neither introduces nor masks a validation fault.
    if validates(&original) != validates(&reloaded) {
        return Outcome::Failed("validation verdict changed after re-encode".to_string());
    }
    Outcome::Passed
}

/// Returns a human-readable description of the first field in which two parsed
/// modules differ, or `None` when they are structurally equal. The instruction
/// stream is diffed element-by-element so a mismatch names the exact index.
fn first_mismatch(original: &ParsedModule, reloaded: &ParsedModule) -> Option<String> {
    if original.name != reloaded.name {
        return Some("name".to_string());
    }
    if original.atoms != reloaded.atoms {
        return Some("atoms".to_string());
    }
    if original.instructions != reloaded.instructions {
        if original.instructions.len() != reloaded.instructions.len() {
            return Some(format!(
                "instruction count {} != {}",
                original.instructions.len(),
                reloaded.instructions.len()
            ));
        }
        for (index, (a, b)) in original
            .instructions
            .iter()
            .zip(reloaded.instructions.iter())
            .enumerate()
        {
            if a != b {
                return Some(format!(
                    "instruction[{index}]:\n    original: {a:?}\n    reloaded: {b:?}"
                ));
            }
        }
        return Some("instructions".to_string());
    }
    if original.imports != reloaded.imports {
        return Some("imports".to_string());
    }
    if original.exports != reloaded.exports {
        return Some("exports".to_string());
    }
    if original.lambdas != reloaded.lambdas {
        return Some(format!(
            "lambdas:\n    original: {:?}\n    reloaded: {:?}",
            original.lambdas, reloaded.lambdas
        ));
    }
    if original.literals != reloaded.literals {
        if original.literals.len() != reloaded.literals.len() {
            return Some(format!(
                "literal count {} != {}",
                original.literals.len(),
                reloaded.literals.len()
            ));
        }
        for (index, (a, b)) in original
            .literals
            .iter()
            .zip(reloaded.literals.iter())
            .enumerate()
        {
            if a != b {
                return Some(format!(
                    "literal[{index}]:\n    original: {a:?}\n    reloaded: {b:?}"
                ));
            }
        }
        return Some("literals".to_string());
    }
    if original.string_table != reloaded.string_table {
        return Some("string_table".to_string());
    }
    if original.line_info != reloaded.line_info {
        return Some("line_info".to_string());
    }
    None
}

/// `validate_module` verdict for a parsed module, resolving imports against an
/// empty registry (external imports become deferred, which validate accepts).
fn validates(module: &ParsedModule) -> bool {
    let registry = ModuleRegistry::new();
    let (resolved, _report) = resolve_imports(module, &registry, &NoBifs, &AllCapabilitiesPolicy);
    validate_module(module, &resolved).is_ok()
}

#[test]
fn every_fixture_round_trips_through_encode() {
    let fixtures = all_fixtures();
    assert!(
        fixtures.len() >= 70,
        "expected the full fixture corpus, found {}",
        fixtures.len()
    );

    let mut failures = Vec::new();
    for path in &fixtures {
        let bytes = std::fs::read(path).expect("fixture readable");
        match drive(&bytes) {
            Outcome::Passed => {}
            Outcome::Excluded(reason) => {
                // A committed fixture that the decoder itself rejects is a
                // regression in the fixture corpus, not an acceptable skip.
                failures.push(format!(
                    "{}: decoder rejected fixture: {reason}",
                    path.display()
                ));
            }
            Outcome::Failed(detail) => {
                failures.push(format!("{}: {detail}", path.display()));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} committed fixtures failed to round-trip:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

/// Full-corpus ratchet: when `BEAMR_RT_CORPUS_DIR` names a directory, every
/// `*.beam` beneath it is driven through `decode -> encode -> decode`. Failures
/// are collected with their paths (the walk never stops at the first), and the
/// module counts are printed loudly so a green run still proves what it covered.
///
/// Undecodable inputs are *excluded* (the encoder is never reached) and reported
/// separately — they are not encoder bugs. Every module the decoder accepts must
/// round-trip through the encoder byte-for-structure identical, or the test
/// fails naming each offender.
#[test]
fn corpus_round_trips_when_env_set() {
    let Ok(dir) = std::env::var("BEAMR_RT_CORPUS_DIR") else {
        println!(
            "BEAMR_RT_CORPUS_DIR unset — corpus ratchet skipped \
             (committed-fixture ratchet still runs in the sibling test)"
        );
        return;
    };

    let root = PathBuf::from(&dir);
    let mut beams = Vec::new();
    collect_beams(&root, &mut beams);
    beams.sort();

    assert!(
        !beams.is_empty(),
        "BEAMR_RT_CORPUS_DIR={dir} contained no *.beam files"
    );

    let mut passed = 0_usize;
    let mut failures = Vec::new();
    let mut excluded = Vec::new();
    for path in &beams {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{}: unreadable: {error}", path.display()));
                continue;
            }
        };
        match drive(&bytes) {
            Outcome::Passed => passed += 1,
            Outcome::Excluded(reason) => excluded.push(format!("{}: {reason}", path.display())),
            Outcome::Failed(detail) => failures.push(format!("{}\n  {detail}", path.display())),
        }
    }

    println!("=== corpus round-trip ratchet: {dir} ===");
    println!("  discovered .beam files : {}", beams.len());
    println!("  encoder round-tripped  : {passed}");
    println!("  excluded (undecodable) : {}", excluded.len());
    println!("  encoder failures       : {}", failures.len());
    if !excluded.is_empty() {
        println!("--- excluded (decoder declined; encoder not exercised) ---");
        for line in &excluded {
            println!("  EXCLUDED {line}");
        }
    }
    if !failures.is_empty() {
        println!("--- encoder failures ---");
        for line in &failures {
            println!("  FAILED {line}");
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} decodable corpus modules failed to round-trip through the encoder",
        failures.len(),
        passed + failures.len()
    );
}

/// A hand-built module carrying the nasty operand and literal shapes: negative
/// and bignum integers, atom-index width boundaries, an empty string table, and
/// a multi-entry `LitT` in the zero-prefix uncompressed form.
#[test]
fn hand_built_edge_cases_round_trip() {
    let table = AtomTable::with_common_atoms();
    let module_name = table.intern("edge_module");
    let start = table.intern("start");
    let wide = table.intern("wide_atom");

    // A big-integer literal beyond i64 (magnitude 2^70), sign byte + LE bytes.
    let mut big_magnitude = vec![0_u8; 9];
    big_magnitude[8] = 0x40; // 2^70 == 1 << 70 -> byte 8 (bit 6) set.
    let mut big_integer_bytes = vec![0_u8]; // positive sign
    big_integer_bytes.extend_from_slice(&big_magnitude);

    let literals = vec![
        Literal::Integer(-1),
        Literal::Integer(i64::MIN),
        Literal::BigInteger(big_integer_bytes),
        Literal::Tuple(vec![
            Literal::Atom(start),
            Literal::String(b"hello".to_vec()),
            Literal::Float(3.5),
        ]),
        Literal::Nil,
        Literal::Binary(vec![1, 2, 3, 4]),
    ];

    let instructions = vec![
        Instruction::Label { label: 1 },
        Instruction::FuncInfo {
            module: Operand::Atom(Some(module_name)),
            function: Operand::Atom(Some(start)),
            arity: Operand::Unsigned(0),
        },
        Instruction::Label { label: 2 },
        // Negative and large inline integer operands.
        Instruction::Move {
            source: Operand::Integer(-42),
            destination: Operand::X(0),
        },
        Instruction::Move {
            source: Operand::Integer(40000),
            destination: Operand::X(1),
        },
        // A literal reference into the LitT table.
        Instruction::Move {
            source: Operand::Literal(2),
            destination: Operand::X(2),
        },
        // Wide atom operand + a synthetic is_function2 (list-wrapped operands).
        Instruction::Move {
            source: Operand::Atom(Some(wide)),
            destination: Operand::X(3),
        },
        Instruction::TypeTest {
            op: TypeTestOp::IsFunction2,
            fail: Operand::Label(2),
            value: Operand::List(vec![Operand::X(3), Operand::Integer(2)]),
        },
        Instruction::Return,
    ];

    let module = ParsedModule {
        name: module_name,
        atoms: vec![module_name, start, wide],
        instructions,
        imports: Vec::new(),
        exports: vec![ExportEntry {
            function: start,
            arity: 0,
            label: 1,
        }],
        lambdas: Vec::new(),
        literals,
        string_table: Vec::new(),
        line_info: Vec::new(),
    };

    let encoded = encode_module(&module, &table).expect("edge module encodes");
    let reloaded = load_beam_chunks(&encoded, &table).expect("edge module decodes");
    assert_eq!(module, reloaded);
}

/// An empty `LitT` (no literals) omits the chunk entirely and still round-trips
/// to an empty literal table.
#[test]
fn empty_optional_chunks_round_trip() {
    let table = AtomTable::with_common_atoms();
    let name = table.intern("bare_module");
    let module = ParsedModule {
        name,
        atoms: vec![name],
        instructions: vec![Instruction::Label { label: 1 }, Instruction::Return],
        imports: Vec::new(),
        exports: Vec::new(),
        lambdas: Vec::new(),
        literals: Vec::new(),
        string_table: Vec::new(),
        line_info: Vec::new(),
    };

    let encoded = encode_module(&module, &table).expect("bare module encodes");
    let reloaded = load_beam_chunks(&encoded, &table).expect("bare module decodes");
    assert_eq!(module, reloaded);
    assert!(reloaded.literals.is_empty());
}
