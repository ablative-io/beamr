//! WPORT-5 R1 registry seal — the profile document cannot drift silently.
//!
//! Builds the browser BIF registry through the REAL wrapper composition
//! (`beamr_wasm::build_wasm_safe_registry`, the `#[doc(hidden)]` wrapper of
//! the private `register_wasm_safe_bifs`) and asserts EXACT two-way set
//! equality between the registered `(module, function, arity)` set and the
//! machine-readable row keys of `docs/design/beamr/BROWSER-BIF-PROFILE.md`,
//! plus the exact row count. A registered-but-unlisted MFA and a
//! listed-but-unregistered row BOTH fail.
//!
//! Recomposing the registry in-test from beamr's public registration chains
//! is FORBIDDEN (OQ6 rider): it would forfeit exactly the cannot-drift
//! property this seal exists for — see the doc-comments on
//! `build_wasm_safe_registry` and `BifRegistryImpl::registered_mfas`.
//!
//! Runs as a plain native test (precedent: `tests/generated_bootstrap.rs`)
//! on the workspace `--test '*'` leg, NOT in the pinned Node count.

use std::collections::BTreeSet;
use std::path::Path;

/// The sealed static registration total (74 gate1 + 17 gate2 + 106 stdlib).
const SEALED_ROW_COUNT: usize = 197;

const SEAL_BEGIN: &str = "<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->";
const SEAL_END: &str = "<!-- SEAL:END REGISTERED-MFA-TABLE -->";

fn profile_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/design/beamr/BROWSER-BIF-PROFILE.md")
}

/// Parse the machine-readable row keys: inside sealed regions, every table
/// line beginning ``| ` `` carries exactly one `module:function/arity` key in
/// its first cell. Sub-rows (`| ↳`) are not keys. Duplicate keys fail.
fn parse_profile_keys(document: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut inside = false;
    for (number, line) in document.lines().enumerate() {
        if line.trim() == SEAL_BEGIN {
            assert!(!inside, "nested seal-begin at line {}", number + 1);
            inside = true;
            continue;
        }
        if line.trim() == SEAL_END {
            assert!(inside, "unmatched seal-end at line {}", number + 1);
            inside = false;
            continue;
        }
        if !inside || !line.starts_with("| `") {
            continue;
        }
        let rest = &line[3..];
        let key = rest
            .split('`')
            .next()
            .unwrap_or_else(|| panic!("malformed sealed row at line {}: {line}", number + 1));
        assert!(
            key.contains(':') && key.contains('/'),
            "sealed row key {key:?} at line {} is not module:function/arity",
            number + 1
        );
        assert!(
            keys.insert(key.to_owned()),
            "duplicate sealed row key {key:?} at line {}",
            number + 1
        );
    }
    assert!(!inside, "unterminated seal region");
    keys
}

fn registered_keys() -> BTreeSet<String> {
    let (registry, atom_table) =
        beamr_wasm::build_wasm_safe_registry().expect("real wrapper composition registers");
    registry
        .registered_mfas()
        .into_iter()
        .map(|(module, function, arity, _capability)| {
            let module = atom_table.resolve(module).expect("module atom resolves");
            let function = atom_table
                .resolve(function)
                .expect("function atom resolves");
            format!("{module}:{function}/{arity}")
        })
        .collect()
}

#[test]
fn profile_rows_exactly_match_the_registered_browser_surface() {
    let document = std::fs::read_to_string(profile_path()).expect("profile document exists");
    let listed = parse_profile_keys(&document);
    let registered = registered_keys();

    let registered_but_unlisted: Vec<_> = registered.difference(&listed).collect();
    let listed_but_unregistered: Vec<_> = listed.difference(&registered).collect();
    assert!(
        registered_but_unlisted.is_empty(),
        "registered but MISSING from the profile document: {registered_but_unlisted:?}"
    );
    assert!(
        listed_but_unregistered.is_empty(),
        "listed in the profile document but NOT registered: {listed_but_unregistered:?}"
    );
    assert_eq!(
        registered.len(),
        SEALED_ROW_COUNT,
        "registered browser surface count moved; update the profile AND this seal deliberately"
    );
    assert_eq!(listed.len(), SEALED_ROW_COUNT);
}

/// Proof by construction (R1 acceptance): a synthetic MFA registered on top
/// of the real composition, with no document row, breaks set equality — a
/// registered-but-unlisted BIF cannot pass the seal.
#[test]
fn synthetic_registration_without_a_row_breaks_the_seal() {
    let document = std::fs::read_to_string(profile_path()).expect("profile document exists");
    let listed = parse_profile_keys(&document);

    let (registry, atom_table) =
        beamr_wasm::build_wasm_safe_registry().expect("real wrapper composition registers");
    let module = atom_table.intern("wport5_seal_probe");
    let function = atom_table.intern("synthetic");
    registry
        .register(
            module,
            function,
            0,
            synthetic_bif,
            beamr::native::Capability::Pure,
        )
        .expect("synthetic registration succeeds");

    let registered: BTreeSet<String> = registry
        .registered_mfas()
        .into_iter()
        .map(|(module, function, arity, _capability)| {
            format!(
                "{}:{}/{arity}",
                atom_table.resolve(module).expect("module atom resolves"),
                atom_table
                    .resolve(function)
                    .expect("function atom resolves"),
            )
        })
        .collect();

    let registered_but_unlisted: Vec<_> = registered.difference(&listed).collect();
    assert_eq!(
        registered_but_unlisted,
        vec![&"wport5_seal_probe:synthetic/0".to_owned()],
        "the synthetic MFA must be the exact equality break the seal detects"
    );
}

fn synthetic_bif(
    _args: &[beamr::term::Term],
    _context: &mut beamr::native::ProcessContext,
) -> Result<beamr::term::Term, beamr::term::Term> {
    Ok(beamr::term::Term::NIL)
}
