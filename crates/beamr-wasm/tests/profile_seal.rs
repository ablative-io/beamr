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

/// The sealed static registration total (74 gate1 + 17 gate2 + 106 stdlib
/// + 5 WPORT-8 capability adapters).
const SEALED_ROW_COUNT: usize = 202;

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

/// Entries registered ONLY on feature-unified native builds — cfg-gated OUT
/// of the wasm closure at the source level (the net/fs blocks of
/// `crates/beamr/src/native/bifs.rs` plus the `global`/`pg` distribution
/// modules). The browser closure (`cooperative,json`, `--locked`) can never
/// register them; a native `--all-features` workspace build feature-unifies
/// beamr with `net`+`fs`, so the REAL wrapper composition sweeps them in on
/// that build only. This is an ALLOWLIST for that build shape: an extra
/// entry NOT on it — a new chain or a filter change inside
/// `register_wasm_safe_bifs` — still fails the seal on every build.
fn cfg_gated_out_of_the_wasm_closure(key: &str) -> bool {
    if key.starts_with("global:") || key.starts_with("pg:") {
        return true;
    }
    const NETFS_ERLANG_KEYS: &[&str] = &[
        "erlang:close_file/1",
        "erlang:del_dir/1",
        "erlang:del_file/1",
        "erlang:file_info/1",
        "erlang:file_seek/3",
        "erlang:inet_close/1",
        "erlang:inet_getopts/2",
        "erlang:inet_peername/1",
        "erlang:inet_port/1",
        "erlang:inet_setopts/2",
        "erlang:inet_sockname/1",
        "erlang:list_dir/1",
        "erlang:make_dir/1",
        "erlang:open_file/2",
        "erlang:pread/3",
        "erlang:pwrite/3",
        "erlang:read_file/2",
        "erlang:rename/2",
        "erlang:tcp_accept/1",
        "erlang:tcp_accept/2",
        "erlang:tcp_connect/3",
        "erlang:tcp_controlling_process/2",
        "erlang:tcp_listen/2",
        "erlang:tcp_recv/2",
        "erlang:tcp_recv/3",
        "erlang:tcp_send/2",
        "erlang:tcp_setopts/2",
        "erlang:udp_open/1",
        "erlang:udp_open/2",
        "erlang:udp_recv/2",
        "erlang:udp_recv/3",
        "erlang:udp_send/4",
        "erlang:write_file/2",
    ];
    NETFS_ERLANG_KEYS.contains(&key)
}

/// Split the composition's extras (registered − listed) into the cfg-gated
/// feature-unification set and genuine seal breaks.
fn split_extras<'set>(
    registered: &'set BTreeSet<String>,
    listed: &'set BTreeSet<String>,
) -> (Vec<&'set String>, Vec<&'set String>) {
    registered
        .difference(listed)
        .partition(|key| cfg_gated_out_of_the_wasm_closure(key))
}

#[test]
fn profile_rows_exactly_match_the_registered_browser_surface() {
    let document = std::fs::read_to_string(profile_path()).expect("profile document exists");
    let listed = parse_profile_keys(&document);
    let registered = registered_keys();

    // BOTH directions, on every build shape:
    // - a listed-but-unregistered row always fails;
    // - a registered-but-unlisted entry fails unless it is one of the
    //   cfg-gated net/fs/global/pg entries that ONLY a feature-unified
    //   native build sweeps into the real composition (see
    //   `cfg_gated_out_of_the_wasm_closure`); on the browser closure the
    //   gated set is empty and the equality is exact.
    let (gated_extras, seal_breaks) = split_extras(&registered, &listed);
    let listed_but_unregistered: Vec<_> = listed.difference(&registered).collect();
    assert!(
        seal_breaks.is_empty(),
        "registered but MISSING from the profile document: {seal_breaks:?}"
    );
    assert!(
        listed_but_unregistered.is_empty(),
        "listed in the profile document but NOT registered: {listed_but_unregistered:?}"
    );
    assert_eq!(
        registered.len() - gated_extras.len(),
        SEALED_ROW_COUNT,
        "registered browser surface count moved; update the profile AND this seal deliberately"
    );
    assert_eq!(listed.len(), SEALED_ROW_COUNT);
    if !gated_extras.is_empty() {
        // Loud, never silent: this build shape is a feature-unified NATIVE
        // test build, not the browser closure. The browser closure is
        // additionally sealed by the default-feature `profile_seal` leg.
        println!(
            "note: feature-unified build — {} cfg-gated net/fs/global/pg entries present \
             beyond the browser surface",
            gated_extras.len()
        );
    }
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

    let (_gated_extras, seal_breaks) = split_extras(&registered, &listed);
    assert_eq!(
        seal_breaks,
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
