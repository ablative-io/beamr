//! `elixir_reach_probe` — a read-only measurement probe over beamr's loader.
//!
//! ```text
//! cargo run -p beamr --example elixir_reach_probe -- <module.beam> [--dir <ebin-dir>]...
//! ```
//!
//! Loads the target `.beam` under exactly the native registry the `beamr` CLI
//! builds in `load_context` (`crates/beamr-cli/src/main.rs:380-432`), with the
//! same `--dir` pre-load policy as that file's `load_beam_dir`
//! (`main.rs:442-484`), and prints ONE JSON object to stdout. The embedded
//! module branch (`embedded_module_bytes`) is deliberately NOT replicated: the
//! probe always reads the file named on the command line, so what it measures
//! is the file, never an embedded namesake.
//!
//! The JSON is written by hand — no `serde_json`, so the probe builds under
//! beamr's default features with the `json` feature OFF.
//!
//! Exit status: 0 whether the module loads or not (a refusal IS the
//! measurement); 2 only on a usage error, an unreadable target, an unreadable
//! `--dir`, or a native-registration fault.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use beamr::atom::AtomTable;
use beamr::error::LoadError;
use beamr::loader::{Instruction, UnresolvedImportReport, load_module_with_origin};
use beamr::module::{ModuleOrigin, ModuleRegistry};
use beamr::native::{
    BifRegistryImpl,
    bifs::register_gate1_bifs,
    gate3_bifs::register_gate3_bifs,
    gleam_ffi::register_gleam_ffi_bifs,
    meridian_ffi::register_meridian_ffi,
    otp_stubs::{init_otp_atoms, register_otp_stubs},
    process_bifs::register_gate2_bifs,
    stdlib_stubs::register_stdlib_stubs,
};
use beamr::term::{Term, format::format_term};

const USAGE: &str = "Usage:\n  cargo run -p beamr --example elixir_reach_probe -- <module.beam> [--dir <ebin-dir>]...\n  cargo run -p beamr --example elixir_reach_probe -- --natives";

/// Native registrations from the CLI's `load_context` that this probe could
/// NOT replicate because the function is not public from the `beamr` library
/// crate. Every one of the eight is public at the same path `beamr-cli`
/// imports it from, so this list is empty; it stays in the JSON so a future
/// divergence is visible in the measurement rather than silent.
const REGISTRATION_DEVIATION: &[&str] = &[];

fn main() -> ExitCode {
    match run() {
        Ok(json) => {
            print!("{json}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("elixir_reach_probe: {message}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

struct Args {
    path: PathBuf,
    dirs: Vec<PathBuf>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut path: Option<PathBuf> = None;
    let mut dirs: Vec<PathBuf> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--dir requires a directory path".to_owned())?;
                dirs.push(PathBuf::from(value));
            }
            "--help" | "-h" => return Err("help requested".to_owned()),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if path.is_some() {
                    return Err(format!("unexpected extra argument {other}"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    let path = path.ok_or_else(|| "missing <module.beam>".to_owned())?;
    Ok(Args { path, dirs })
}

fn run() -> Result<String, String> {
    // `--natives` mode: print the registry's own registered MFAs (one
    // `module:function/arity` per line, sorted) and exit — the native census
    // for axis D, read from the registry rather than from source tables.
    if std::env::args().nth(1).as_deref() == Some("--natives") {
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let bif_registry = build_registry(&atom_table)?;
        let mut lines: Vec<String> = bif_registry
            .registered_mfas()
            .into_iter()
            .map(|(module, function, arity, _)| {
                format!(
                    "{}:{}/{arity}",
                    format_term(Term::atom(module), &atom_table),
                    format_term(Term::atom(function), &atom_table)
                )
            })
            .collect();
        lines.sort();
        lines.dedup();
        let mut out = String::new();
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        return Ok(out);
    }
    let args = parse_args(std::env::args().skip(1))?;

    // Read the target BEFORE building the registry: an unreadable target is a
    // usage-class fault (exit 2), not a load measurement.
    let bytes = std::fs::read(&args.path)
        .map_err(|error| format!("cannot read {}: {error}", args.path.display()))?;

    // ── native registry, in the CLI's exact order (main.rs:380-432) ───────
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let bif_registry = build_registry(&atom_table)?;
    let module_registry = ModuleRegistry::new();

    // Directories load first so the target's imports can resolve against them.
    let mut skipped_in_dirs = 0usize;
    for dir in &args.dirs {
        skipped_in_dirs += load_beam_dir(dir, &atom_table, &module_registry, &bif_registry)?;
    }

    let outcome = load_module_with_origin(
        &bytes,
        &atom_table,
        &module_registry,
        &*bif_registry,
        ModuleOrigin::Filesystem(args.path.clone()),
    );

    Ok(render_json(&args, skipped_in_dirs, outcome, &atom_table))
}

fn registration_failure(error: beamr::native::NativeRegistrationError) -> String {
    format!("native registration failed: {error}")
}

/// Loads every `.beam` file in `dir` into the module registry, replicating the
/// CLI's `load_beam_dir` policy (main.rs:442-484): files that fail to read or
/// decode are skipped — they may be modules with unsupported features that are
/// not needed — but every skip is reported on stderr (never stdout: the JSON
/// object on stdout must stay parseable) with the file and reason, plus an
/// aggregate count. Returns the number skipped.
/// The CLI's native registry, built in `load_context`'s exact order
/// (main.rs:380-395).
fn build_registry(atom_table: &Arc<AtomTable>) -> Result<Arc<BifRegistryImpl>, String> {
    let bif_registry = Arc::new(BifRegistryImpl::new());
    register_gate1_bifs(&bif_registry, atom_table).map_err(registration_failure)?;
    register_gate2_bifs(&bif_registry, atom_table).map_err(registration_failure)?;
    register_gate3_bifs(&bif_registry, atom_table).map_err(registration_failure)?;
    register_stdlib_stubs(&bif_registry, atom_table).map_err(registration_failure)?;
    register_gleam_ffi_bifs(&bif_registry, atom_table).map_err(registration_failure)?;
    register_meridian_ffi(&bif_registry, atom_table).map_err(registration_failure)?;
    init_otp_atoms(atom_table);
    register_otp_stubs(&bif_registry, atom_table).map_err(registration_failure)?;
    Ok(bif_registry)
}

fn load_beam_dir(
    dir: &Path,
    atom_table: &AtomTable,
    module_registry: &ModuleRegistry,
    bif_registry: &BifRegistryImpl,
) -> Result<usize, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read directory {}: {error}", dir.display()))?;
    let mut skipped = 0usize;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read directory {}: {error}", dir.display()))?;
        let file_path = entry.path();
        if file_path.extension().is_some_and(|ext| ext == "beam") {
            let bytes = match std::fs::read(&file_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("beamr: warning: skipped {}: {error}", file_path.display());
                    skipped += 1;
                    continue;
                }
            };
            if let Err(error) = load_module_with_origin(
                &bytes,
                atom_table,
                module_registry,
                bif_registry,
                ModuleOrigin::Filesystem(file_path.clone()),
            ) {
                eprintln!("beamr: warning: skipped {}: {error}", file_path.display());
                skipped += 1;
            }
        }
    }
    if skipped > 0 {
        eprintln!(
            "beamr: warning: skipped {skipped} .beam file(s) in {}",
            dir.display()
        );
    }
    Ok(skipped)
}

/// The counts of `Instruction::Generic` by name. The three names built by
/// `loader/decode/code.rs` are `set_tuple_element` (67), `executable_line`
/// (183) and `debug_line` (184); anything else lands in `other`.
#[derive(Default)]
struct GenericHistogram {
    set_tuple_element: usize,
    executable_line: usize,
    debug_line: usize,
    other: usize,
}

fn histogram(instructions: &[Instruction]) -> GenericHistogram {
    let mut counts = GenericHistogram::default();
    for instruction in instructions {
        if let Instruction::Generic { name, .. } = instruction {
            match *name {
                "set_tuple_element" => counts.set_tuple_element += 1,
                "executable_line" => counts.executable_line += 1,
                "debug_line" => counts.debug_line += 1,
                _ => counts.other += 1,
            }
        }
    }
    counts
}

/// Extracts N from a decode error message of the exact shape
/// `unsupported opcode N` (loader/decode/code.rs:427-431). Returns `None` for
/// every other decode failure, which is what keeps `load_status` honest:
/// only an opcode refusal is reported as `decode-failed`.
fn unsupported_opcode(message: &str) -> Option<u32> {
    let digits = message.strip_prefix("unsupported opcode ")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Renders `module:function/arity` the way the CLI's `format_import_report`
/// does (main.rs:510-522): `format_term` over `Term::atom`, which resolves the
/// atom's name unquoted, so the line is byte-identical to `beamr imports`.
fn mfa(
    module: beamr::atom::Atom,
    function: beamr::atom::Atom,
    arity: u8,
    table: &AtomTable,
) -> String {
    let mut line = format_term(Term::atom(module), table);
    line.push(':');
    line.push_str(&format_term(Term::atom(function), table));
    line.push('/');
    line.push_str(&arity.to_string());
    line
}

fn import_lines(
    report: &UnresolvedImportReport,
    table: &AtomTable,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut unresolved: Vec<String> = report
        .imports()
        .iter()
        .map(|entry| mfa(entry.module, entry.function, entry.arity, table))
        .collect();
    let mut deferred: Vec<String> = report
        .deferred_imports()
        .iter()
        .map(|entry| mfa(entry.module, entry.function, entry.arity, table))
        .collect();
    let mut denied: Vec<String> = report
        .denied_imports()
        .iter()
        .map(|entry| mfa(entry.module, entry.function, entry.arity, table))
        .collect();
    unresolved.sort();
    deferred.sort();
    denied.sort();
    (unresolved, deferred, denied)
}

type LoadOutcome = Result<(Arc<beamr::module::Module>, UnresolvedImportReport), LoadError>;

fn render_json(
    args: &Args,
    skipped_in_dirs: usize,
    outcome: LoadOutcome,
    atom_table: &AtomTable,
) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"path\": {},\n",
        json_string(&args.path.display().to_string())
    ));
    out.push_str("  \"dirs\": ");
    out.push_str(&json_array(
        &args
            .dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>(),
    ));
    out.push_str(",\n");
    out.push_str(&format!("  \"skipped_in_dirs\": {skipped_in_dirs},\n"));
    out.push_str("  \"registration_deviation\": ");
    out.push_str(&json_array(
        &REGISTRATION_DEVIATION
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    ));
    out.push_str(",\n");

    match outcome {
        Ok((module, report)) => {
            let counts = histogram(&module.code);
            let (unresolved, deferred, denied) = import_lines(&report, atom_table);
            out.push_str("  \"load_status\": \"loaded\",\n");
            out.push_str("  \"load_error\": null,\n");
            out.push_str("  \"first_refused_opcode\": null,\n");
            out.push_str(&format!(
                "  \"module\": {},\n",
                json_string(&format_term(Term::atom(module.name), atom_table))
            ));
            out.push_str(&format!(
                "  \"instruction_count\": {},\n",
                module.code.len()
            ));
            out.push_str(&format!(
                "  \"generic\": {{\"set_tuple_element\": {}, \"executable_line\": {}, \"debug_line\": {}, \"other\": {}}},\n",
                counts.set_tuple_element, counts.executable_line, counts.debug_line, counts.other
            ));
            out.push_str(&format!("  \"unresolved_count\": {},\n", unresolved.len()));
            out.push_str("  \"unresolved\": ");
            out.push_str(&json_array(&unresolved));
            out.push_str(",\n");
            out.push_str(&format!("  \"deferred_count\": {},\n", deferred.len()));
            out.push_str("  \"deferred\": ");
            out.push_str(&json_array(&deferred));
            out.push_str(",\n");
            out.push_str(&format!("  \"denied_count\": {},\n", denied.len()));
            out.push_str("  \"denied\": ");
            out.push_str(&json_array(&denied));
            out.push('\n');
        }
        Err(error) => {
            let display = error.to_string();
            let opcode = match &error {
                LoadError::DecodeError(message) => unsupported_opcode(message),
                _ => None,
            };
            let status = if opcode.is_some() {
                "decode-failed"
            } else {
                "load-failed"
            };
            out.push_str(&format!("  \"load_status\": \"{status}\",\n"));
            out.push_str(&format!("  \"load_error\": {},\n", json_string(&display)));
            match opcode {
                Some(number) => {
                    out.push_str(&format!("  \"first_refused_opcode\": {number},\n"));
                }
                None => out.push_str("  \"first_refused_opcode\": null,\n"),
            }
            // Nothing below is measurable when the module did not load: the
            // instruction stream and the import report only exist on success.
            out.push_str("  \"module\": null,\n");
            out.push_str("  \"instruction_count\": null,\n");
            out.push_str("  \"generic\": null,\n");
            out.push_str("  \"unresolved_count\": null,\n");
            out.push_str("  \"unresolved\": null,\n");
            out.push_str("  \"deferred_count\": null,\n");
            out.push_str("  \"deferred\": null,\n");
            out.push_str("  \"denied_count\": null,\n");
            out.push_str("  \"denied\": null\n");
        }
    }

    out.push_str("}\n");
    out
}

fn json_array(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_owned();
    }
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
    out
}

/// Minimal JSON string escaper: backslash, quote, the named control escapes,
/// and `\u00XX` for every remaining C0 control byte.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < '\u{20}' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
