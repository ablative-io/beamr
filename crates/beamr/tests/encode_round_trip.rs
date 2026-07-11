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

/// Loads the module against `table`, re-encodes it, and reloads it.
fn round_trip(bytes: &[u8], table: &AtomTable) -> (ParsedModule, ParsedModule) {
    let original = load_beam_chunks(bytes, table).expect("fixture decodes");
    let encoded = encode_module(&original, table).expect("module re-encodes");
    let reloaded = load_beam_chunks(&encoded, table).expect("re-encoded module decodes");
    (original, reloaded)
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

    for path in fixtures {
        let bytes = std::fs::read(&path).expect("fixture readable");
        let table = AtomTable::with_common_atoms();
        let (original, reloaded) = round_trip(&bytes, &table);

        assert_eq!(original.name, reloaded.name, "name mismatch in {path:?}");
        assert_eq!(original.atoms, reloaded.atoms, "atoms mismatch in {path:?}");
        assert_eq!(
            original.instructions, reloaded.instructions,
            "instructions mismatch in {path:?}"
        );
        assert_eq!(
            original.imports, reloaded.imports,
            "imports mismatch in {path:?}"
        );
        assert_eq!(
            original.exports, reloaded.exports,
            "exports mismatch in {path:?}"
        );
        assert_eq!(
            original.lambdas, reloaded.lambdas,
            "lambdas mismatch in {path:?}"
        );
        assert_eq!(
            original.literals, reloaded.literals,
            "literals mismatch in {path:?}"
        );
        assert_eq!(
            original.string_table, reloaded.string_table,
            "string table mismatch in {path:?}"
        );
        assert_eq!(
            original.line_info, reloaded.line_info,
            "line info mismatch in {path:?}"
        );
        // Full structural equality (belt and suspenders over the field checks).
        assert_eq!(original, reloaded, "ParsedModule mismatch in {path:?}");

        // The re-encoded module must earn the same validation verdict as the
        // original — encoding neither introduces nor masks a validation fault.
        assert_eq!(
            validates(&original),
            validates(&reloaded),
            "validation verdict changed after re-encode in {path:?}"
        );
    }
}

/// A hand-built module carrying the nasty operand and literal shapes: negative
/// and bignum integers, atom-index width boundaries, an empty string table, and
/// a compressed multi-entry `LitT`.
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
        // A literal reference into the compressed LitT.
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
