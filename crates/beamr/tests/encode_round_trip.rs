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
use beamr::loader::encode::{EncodeError, encode_module};
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
    /// The encoder REFUSED the module by name (#95): it contains typed-register
    /// operands, whose `Type` chunk beamr's decoder drops, so the encoder cannot
    /// represent it faithfully.
    ///
    /// This is neither a pass nor an encoder bug — it is the guard working. It
    /// gets its own variant so the count stays VISIBLE: absorbing refusals into
    /// `Passed` would hide the open defect behind a green run, and classing them
    /// `Failed` would hold the guard hostage to the fix it precedes. When #95's
    /// fix lands and the `Type` chunk is carried, this tally drops to zero.
    Refused(String),
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
        // A typed-register refusal is the #95 guard firing, not an encoder
        // fault. It is matched on the ERROR VALUE, never on the message text,
        // so a reworded message cannot silently reclassify a real failure as an
        // expected refusal.
        Err(EncodeError::TypedRegisterWithoutTypeChunk {
            instruction_index,
            type_index,
        }) => {
            return Outcome::Refused(format!(
                "typed register at instruction {} names Type entry {type_index}",
                instruction_index.map_or_else(|| "?".to_string(), |index| index.to_string())
            ));
        }
        Err(error) => return Outcome::Failed(format!("encode failed: {error}")),
    };
    let reloaded = match load_beam_chunks(&encoded, &table) {
        Ok(module) => module,
        Err(error) => return Outcome::Failed(format!("re-encoded bytes do not decode: {error}")),
    };
    if let Some(detail) = first_mismatch(&original, &reloaded) {
        return Outcome::Failed(format!("round-trip mismatch: {detail}"));
    }
    if let Some(detail) = missing_required_chunks(&encoded) {
        return Outcome::Failed(format!(
            "re-encoded container is not readable by OTP: {detail}"
        ));
    }
    // The re-encoded module must earn the same validation verdict as the
    // original — encoding neither introduces nor masks a validation fault.
    if validates(&original) != validates(&reloaded) {
        return Outcome::Failed("validation verdict changed after re-encode".to_string());
    }
    Outcome::Passed
}

/// The chunks OTP's `beam_lib`/`beam_disasm` hard-require in every module.
///
/// This set was MEASURED, not assumed: each chunk was stripped in turn from a
/// working module and the remainder fed to `beam_disasm` under OTP 29. These
/// five make it refuse the file outright with `{missing_chunk, _, "…"}`; `Attr`,
/// `CInf`, `Dbgi`, `Docs`, `FunT`, `Line`, `LocT`, `Meta` and `Type` are
/// genuinely optional. `LitT` is deliberately absent from this list — stripping
/// it leaves dangling literal references, so that arm of the experiment is a
/// confound rather than evidence, and the encoder still omits it when empty.
const OTP_REQUIRED_CHUNKS: [&[u8; 4]; 5] = [b"AtU8", b"Code", b"ImpT", b"ExpT", b"StrT"];

/// Walks the `FOR1 … BEAM` container and returns its chunk tags in order.
///
/// The container is PARSED — tags are read at the offsets the length fields
/// dictate — rather than searched for as byte patterns, which would match a
/// four-byte sequence occurring inside a chunk body (an atom name or a string
/// literal) and report a chunk that is not there.
fn container_chunk_tags(bytes: &[u8]) -> Result<Vec<[u8; 4]>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"FOR1" || &bytes[8..12] != b"BEAM" {
        return Err("not a FOR1/BEAM container".to_string());
    }
    let declared = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let end = 8usize
        .checked_add(declared)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| {
            format!(
                "declared body length {declared} overruns {} bytes",
                bytes.len()
            )
        })?;

    let mut tags = Vec::new();
    let mut offset = 12; // past FOR1 + size + BEAM
    while offset < end {
        if offset + 8 > end {
            return Err(format!("truncated chunk header at offset {offset}"));
        }
        let tag: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
        let length = u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        tags.push(tag);
        let padded = length.next_multiple_of(4);
        offset = offset
            .checked_add(8 + padded)
            .ok_or_else(|| format!("chunk length {length} overflows the offset"))?;
    }
    if offset != end {
        return Err(format!(
            "chunk walk ended at {offset}, container body ends at {end}"
        ));
    }
    Ok(tags)
}

/// Names the required chunks absent from an encoded container, or `None` when
/// all five are present. A container the walk cannot parse is itself reported.
fn missing_required_chunks(encoded: &[u8]) -> Option<String> {
    let tags = match container_chunk_tags(encoded) {
        Ok(tags) => tags,
        Err(error) => return Some(error),
    };
    let missing: Vec<String> = OTP_REQUIRED_CHUNKS
        .iter()
        .filter(|required| !tags.iter().any(|tag| tag == **required))
        .map(|required| String::from_utf8_lossy(*required).into_owned())
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "missing required chunk(s) {}; emitted {}",
        missing.join(", "),
        tags.iter()
            .map(|tag| String::from_utf8_lossy(tag).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    ))
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
    let mut refused = Vec::new();
    for path in &fixtures {
        let bytes = std::fs::read(path).expect("fixture readable");
        match drive(&bytes) {
            Outcome::Passed => {}
            Outcome::Refused(reason) => {
                refused.push(format!("{}: {reason}", path.display()));
            }
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

    // Stated so a run reports what it declined, not just that nothing failed.
    // ⚠️ The harness CAPTURES this on a passing test — it surfaces on failure or
    // under `--nocapture`, which is how the battery records the tally. It is not
    // visible in an ordinary green `cargo test` run, and claiming otherwise
    // would be the same silent-success shape this whole lane is about.
    println!(
        "=== committed-fixture ratchet: {} fixtures, {} refused (#95 typed registers) ===",
        fixtures.len(),
        refused.len()
    );
    for line in &refused {
        println!("  REFUSED {line}");
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
    let mut refused = Vec::new();
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
            Outcome::Refused(reason) => refused.push(format!("{}: {reason}", path.display())),
            Outcome::Failed(detail) => failures.push(format!("{}\n  {detail}", path.display())),
        }
    }

    println!("=== corpus round-trip ratchet: {dir} ===");
    println!("  discovered .beam files : {}", beams.len());
    println!("  encoder round-tripped  : {passed}");
    println!("  excluded (undecodable) : {}", excluded.len());
    println!("  refused (#95 typed reg): {}", refused.len());
    println!("  encoder failures       : {}", failures.len());
    if !refused.is_empty() {
        println!("--- refused (guard fired; encoder declined by name) ---");
        for line in &refused {
            println!("  REFUSED {line}");
        }
    }
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

/// The five chunks OTP hard-requires are emitted even when their contents are
/// empty — the case our own loader cannot see, because it treats an absent
/// optional chunk as an empty one and round-trips either way.
///
/// The module below is the worst case on purpose: no imports, no exports, no
/// string table, no literals, no lambdas, no line info. Before this gate the
/// encoder omitted `ImpT`, `ExpT` and `StrT` here, and OTP's `beam_disasm`
/// refuses such a file with `{missing_chunk, _, "StrT"}` — which is how a
/// beamr-emitted module went silently unread by an ecosystem-wide sweep.
#[test]
fn required_chunks_are_emitted_even_when_empty() {
    let table = AtomTable::with_common_atoms();
    let name = table.intern("no_content_module");
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

    let encoded = encode_module(&module, &table).expect("empty module encodes");
    let tags = container_chunk_tags(&encoded).expect("encoder emits a walkable container");
    for required in OTP_REQUIRED_CHUNKS {
        assert!(
            tags.iter().any(|tag| tag == required),
            "encoder omitted required chunk {}; emitted {:?}",
            String::from_utf8_lossy(required),
            tags.iter()
                .map(|tag| String::from_utf8_lossy(tag).into_owned())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(missing_required_chunks(&encoded), None);

    // `LitT` stays conditional and is genuinely absent here — the gate asserts a
    // required floor, not that every chunk is always emitted.
    assert!(!tags.iter().any(|tag| tag == b"LitT"));
}

/// Positive control for the gate above: the chunk walk must actually SEE an
/// absent chunk. A presence check that cannot fail proves nothing, so a
/// container is hand-built with `StrT` removed and the walker is required to
/// name it — the exact shape OTP reported for the module that started this.
#[test]
fn the_required_chunk_check_detects_an_absent_chunk() {
    let table = AtomTable::with_common_atoms();
    let name = table.intern("stripped_module");
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
    let encoded = encode_module(&module, &table).expect("module encodes");
    assert_eq!(missing_required_chunks(&encoded), None, "control: intact");

    // Re-frame the container with `StrT` dropped, rebuilding every length field
    // so the result is a well-formed file that is merely missing a chunk.
    let mut body = b"BEAM".to_vec();
    let mut offset = 12;
    while offset < encoded.len() {
        let length =
            u32::from_be_bytes(encoded[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let span = 8 + length.next_multiple_of(4);
        if &encoded[offset..offset + 4] != b"StrT" {
            body.extend_from_slice(&encoded[offset..offset + span]);
        }
        offset += span;
    }
    let mut stripped = b"FOR1".to_vec();
    stripped.extend_from_slice(&(body.len() as u32).to_be_bytes());
    stripped.extend_from_slice(&body);

    let verdict = missing_required_chunks(&stripped).expect("stripped container must be caught");
    assert!(
        verdict.contains("StrT"),
        "the check must name the absent chunk, said: {verdict}"
    );
}
