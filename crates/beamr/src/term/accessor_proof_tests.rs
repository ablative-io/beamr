//! In-gate control for the five `compile_fail` accessor proofs.
//!
//! ⛔ WHY THIS EXISTS — MEASURED, not argued.
//!
//! The five proofs that the retired accessors' bound is a TYPE ERROR
//! (`BigInt::limbs`, `Binary::as_bytes`, `BinaryRef::as_bytes`,
//! `ProcBin::as_bytes`, `SubBinary::as_bytes`) are rustdoc doctests marked
//! ` ```compile_fail,E0502 `. The error code READS like a pin on the
//! diagnostic. On the toolchain this repository pins — `rust-toolchain.toml`,
//! 1.97.1 — IT IS NOT ENFORCED.
//!
//! Measured with an isolated rustdoc control, not inferred: a block whose real
//! error is E0384 passes a ` ```compile_fail,E0308 ` annotation on stable
//! 1.97.1, and the same block FAILS on nightly 1.100.0 with
//! `Some expected error codes were not found: ["E0308"]`. Transcript:
//! `docs/evidence/beamr-accessor-compile-fail-control.txt`.
//!
//! So on the GATED toolchain a `compile_fail` doctest greens on ANY compile
//! error — a missing argument, a mistyped method, an import that does not
//! resolve — while still reading as "the borrow is enforced". Measured on this
//! tree: deleting the `HeapBorrow` argument from one of the five proofs leaves
//! it green, because E0061 is still a compile error. That is the failure mode
//! this module closes, and nothing else in the tree closes it.
//!
//! THE PROOF IS A MATCHED PAIR, and this module is its hinge:
//!
//!   1. the POSITIVE control compiles and RUNS, so every identifier, argument
//!      count and import in the program is real — rustdoc proves that half;
//!   2. this module asserts the `compile_fail` block is the positive control
//!      plus EXACTLY ONE line, and that the line is the collection;
//!   3. the `compile_fail` block does not compile — rustdoc proves that half.
//!
//! Together: the program is well formed, it differs from a working program by
//! one collection call, and that call is what makes it a type error. The
//! conclusion is the borrow bound, and it no longer rests on an annotation the
//! gated toolchain ignores.
//!
//! ⛔ A FAILURE HERE IS NOT AN ASSERTION TO RELAX. It means the two blocks have
//! drifted, and a drifted pair no longer demonstrates what its prose claims.

/// The four sources carrying the five proofs, read at COMPILE time so this
/// control needs no filesystem and runs under every target the gate builds,
/// `wasm32-unknown-unknown` included.
const SOURCES: [(&str, &str); 4] = [
    (
        "term/boxed/accessors.rs",
        include_str!("boxed/accessors.rs"),
    ),
    (
        "term/boxed/binary_accessors.rs",
        include_str!("boxed/binary_accessors.rs"),
    ),
    ("term/binary.rs", include_str!("binary.rs")),
    ("term/binary_ref.rs", include_str!("binary_ref.rs")),
];

/// Every accessor proof in the tree. Asserted on the TOTAL, so deleting one is
/// a failure rather than a quietly smaller population passing every per-pair
/// check it still has.
const EXPECTED_PAIRS: usize = 5;

#[derive(Debug, PartialEq, Eq)]
enum Fence {
    Positive,
    CompileFail,
}

struct Block {
    fence: Fence,
    line: usize,
    body: Vec<String>,
}

/// Classifies a doc-comment fence line, or `None` if the line is not one.
///
/// A positive control must be a BARE fence: `no_run` or `ignore` would stop it
/// being a control, and both make this return `None`, which breaks the
/// alternation the test asserts. The weakening is unrepresentable rather than
/// separately checked.
fn doc_fence(line: &str) -> Option<Fence> {
    let rest = line.trim_start().strip_prefix("/// ```")?;
    if rest.is_empty() {
        Some(Fence::Positive)
    } else if rest.starts_with("compile_fail") {
        Some(Fence::CompileFail)
    } else {
        None
    }
}

/// Strips the doc-comment prefix, leaving the program line as rustdoc sees it.
fn doc_content(line: &str) -> String {
    let trimmed = line.trim_start();
    let stripped = trimmed.strip_prefix("///").unwrap_or(trimmed);
    stripped.strip_prefix(' ').unwrap_or(stripped).to_string()
}

/// Every fenced doc code block in `source`, in file order.
fn blocks(source: &str) -> Vec<Block> {
    let mut found = Vec::new();
    let mut open: Option<Block> = None;
    for (index, line) in source.lines().enumerate() {
        match (open.as_mut(), doc_fence(line)) {
            // A fence while a block is open closes it, whatever it looks like:
            // the closing fence of a `compile_fail` block is a bare ``` too.
            (Some(_), Some(_)) => {
                if let Some(block) = open.take() {
                    found.push(block);
                }
            }
            (None, Some(fence)) => {
                open = Some(Block {
                    fence,
                    line: index + 1,
                    body: Vec::new(),
                });
            }
            (Some(block), None) => block.body.push(doc_content(line)),
            (None, None) => {}
        }
    }
    found
}

/// Asserts `negative` is `positive` with exactly one line inserted, and that
/// the inserted line is the collection.
fn assert_differs_by_the_collection(name: &str, positive: &Block, negative: &Block) {
    assert_eq!(
        negative.body.len(),
        positive.body.len() + 1,
        "{name}:{} — the compile_fail proof must be its positive control plus \
         exactly one line, but it carries {} lines against the control's {}",
        negative.line,
        negative.body.len(),
        positive.body.len()
    );
    let split = positive
        .body
        .iter()
        .zip(negative.body.iter())
        .position(|(control, proof)| control != proof)
        .unwrap_or(positive.body.len());
    assert!(
        negative.body[split].contains("collect_minor"),
        "{name}:{} — the one line the compile_fail proof adds must be the \
         collection, but it is {:?}",
        negative.line,
        negative.body[split]
    );
    assert_eq!(
        negative.body[..split],
        positive.body[..split],
        "{name}:{} — the proof and its control diverge before the collection",
        negative.line
    );
    assert_eq!(
        negative.body[split + 1..],
        positive.body[split..],
        "{name}:{} — the proof and its control diverge after the collection",
        negative.line
    );
}

#[test]
fn every_compile_fail_proof_is_its_positive_control_plus_the_collection() {
    let mut pairs = 0;
    for (name, source) in SOURCES {
        let found = blocks(source);
        assert!(
            found.len().is_multiple_of(2),
            "{name}: {} doc code blocks — the proofs come in pairs, so an odd \
             count means one lost its positive control",
            found.len()
        );
        for pair in found.chunks(2) {
            let (control, proof) = (&pair[0], &pair[1]);
            assert_eq!(
                control.fence,
                Fence::Positive,
                "{name}:{} — a proof pair must open with a bare, runnable \
                 positive control",
                control.line
            );
            assert_eq!(
                proof.fence,
                Fence::CompileFail,
                "{name}:{} — the second block of a pair must be the \
                 compile_fail proof",
                proof.line
            );
            assert_differs_by_the_collection(name, control, proof);
            pairs += 1;
        }
    }
    assert_eq!(
        pairs, EXPECTED_PAIRS,
        "expected {EXPECTED_PAIRS} accessor proof pairs, found {pairs} — a \
         proof was added or deleted without this population being updated"
    );
}
