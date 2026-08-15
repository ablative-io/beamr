//! JIT rejection census — REAL-ERLC-ADMISSION Leg 2 instrument.
//!
//! Read-only: walks real BEAM corpora through `beamr::jit::aot::AotCompiler`
//! (the same slicer + pre-pass + Cranelift lowering the demand path uses) and
//! records, per exported function, COMPILED or SKIPPED(reason).
//!
//! Discipline: no stderr suppression, no parallelism, no silent swallowing.
//! Every .beam file discovered gets exactly one row in the modules TSV.

use beamr::atom::{Atom, AtomTable};
use beamr::jit::aot::{AotCompiler, AotError};
use beamr::loader::load_beam_chunks;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

const FIXTURE_ROOT: &str = "/Users/tom/Developer/ablative/stack/beamr/crates/beamr/tests/fixtures";
const OTP_LIB_ROOT: &str = "/opt/homebrew/Cellar/erlang/29.0.3/lib/erlang/lib";

/// One row per exported function.
struct ExportRow {
    corpus: &'static str,
    app: String,
    file: String,
    function: String,
    arity: u8,
    outcome: &'static str,
    raw_reason: String,
}

/// One row per `.beam` file.
struct ModuleRow {
    corpus: &'static str,
    app: String,
    file: String,
    module_outcome: String,
    n_exports: usize,
    n_compiled: usize,
    n_skipped: usize,
}

/// A discovered corpus member.
struct Found {
    corpus: &'static str,
    app: String,
    path: PathBuf,
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("usage: jit-rejection-census <output-dir>"));
    let out_dir = PathBuf::from(out_dir);

    // Panic hook: record the message AND keep the default stderr print.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Ok(mut slot) = LAST_PANIC.lock() {
            *slot = Some(format!("{info}").replace(['\n', '\r', '\t'], " "));
        }
        default_hook(info);
    }));

    // Smoke check: the JIT backend can be instantiated at all.
    match AotCompiler::new() {
        Ok(_) => println!("[setup] AotCompiler::new() OK"),
        Err(error) => {
            println!("[setup] FATAL: AotCompiler::new() failed: {error}");
            std::process::exit(2);
        }
    }

    let fixtures = discover_fixtures();
    let otp = discover_otp();
    println!(
        "[discover] FIXTURES files-found = {}\n[discover] OTP_STDLIB files-found = {}",
        fixtures.len(),
        otp.len()
    );
    let files_found_total = fixtures.len() + otp.len();

    let mut export_rows: Vec<ExportRow> = Vec::new();
    let mut module_rows: Vec<ModuleRow> = Vec::new();

    // ---- FIXTURES leg (also carries the two self-tests) ----
    println!("[sweep] FIXTURES begin");
    sweep(&fixtures, &mut export_rows, &mut module_rows);
    println!("[sweep] FIXTURES done: {} module rows", module_rows.len());

    let fixture_compiled = export_rows
        .iter()
        .filter(|r| r.corpus == "FIXTURES" && r.outcome == "COMPILED")
        .count();
    let fixture_skipped = export_rows
        .iter()
        .filter(|r| r.corpus == "FIXTURES" && r.outcome == "SKIPPED")
        .count();

    let mut anomalies: Vec<String> = Vec::new();
    if fixture_compiled == 0 {
        let line = "SELF-TEST ANOMALY (positive control): ZERO fixture functions COMPILED. \
             The sweep continues, but every COMPILED count below is suspect."
            .to_owned();
        println!("\n!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
        println!("{line}");
        println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!\n");
        anomalies.push(line);
    } else {
        println!("[self-test] positive control PASS: {fixture_compiled} fixture functions COMPILED");
    }
    if fixture_skipped == 0 {
        let line = "SELF-TEST ANOMALY (negative control): ZERO fixture functions SKIPPED. \
             The rejection instrument may be inert."
            .to_owned();
        println!("\n!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
        println!("{line}");
        println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!\n");
        anomalies.push(line);
    } else {
        println!("[self-test] negative control PASS: {fixture_skipped} fixture functions SKIPPED");
    }

    // ---- OTP leg ----
    println!("[sweep] OTP_STDLIB begin ({} files)", otp.len());
    sweep(&otp, &mut export_rows, &mut module_rows);
    println!("[sweep] OTP_STDLIB done");

    // ---- Conservation assertion ----
    println!(
        "[assert] files-found = {files_found_total}, module rows = {}",
        module_rows.len()
    );
    assert_eq!(
        files_found_total,
        module_rows.len(),
        "CONSERVATION FAILURE: every discovered .beam must appear exactly once in the modules TSV"
    );
    let mut seen_paths: HashSet<&str> = HashSet::new();
    for row in &module_rows {
        assert!(
            seen_paths.insert(row.file.as_str()),
            "CONSERVATION FAILURE: duplicate module row for {}",
            row.file
        );
    }
    println!("[assert] conservation OK (counts equal, no duplicate paths)");

    write_raw_tsv(&out_dir.join("jit-census-raw.tsv"), &export_rows);
    write_module_tsv(&out_dir.join("jit-census-modules.tsv"), &module_rows);
    write_ranking(
        &out_dir.join("jit-census-ranking.md"),
        &export_rows,
        &module_rows,
        files_found_total,
        &anomalies,
    );
    println!("[done] outputs written to {}", out_dir.display());
}

// ---------------------------------------------------------------- discovery

fn discover_fixtures() -> Vec<Found> {
    let root = Path::new(FIXTURE_ROOT);
    let mut acc = Vec::new();
    walk_beams(root, &mut acc);
    acc.sort();
    acc.into_iter()
        .map(|path| {
            let app = path
                .strip_prefix(root)
                .ok()
                .and_then(|rel| rel.parent())
                .map(|parent| parent.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_owned());
            Found {
                corpus: "FIXTURES",
                app,
                path,
            }
        })
        .collect()
}

fn discover_otp() -> Vec<Found> {
    let root = Path::new(OTP_LIB_ROOT);
    let mut apps: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot read OTP lib root {}: {error}", root.display()))
        .map(|entry| entry.expect("OTP lib dir entry").path())
        .filter(|path| path.is_dir())
        .collect();
    apps.sort();

    let mut out = Vec::new();
    for app_dir in apps {
        let app = app_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".to_owned());
        let ebin = app_dir.join("ebin");
        if !ebin.is_dir() {
            continue;
        }
        let mut beams: Vec<PathBuf> = std::fs::read_dir(&ebin)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", ebin.display()))
            .map(|entry| entry.expect("ebin entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "beam"))
            .collect();
        beams.sort();
        for path in beams {
            out.push(Found {
                corpus: "OTP_STDLIB",
                app: app.clone(),
                path,
            });
        }
    }
    out
}

fn walk_beams(dir: &Path, acc: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            walk_beams(&path, acc);
        } else if path.extension().is_some_and(|ext| ext == "beam") {
            acc.push(path);
        }
    }
}

// ------------------------------------------------------------------- sweep

fn sweep(files: &[Found], export_rows: &mut Vec<ExportRow>, module_rows: &mut Vec<ModuleRow>) {
    for (index, found) in files.iter().enumerate() {
        let file = found.path.to_string_lossy().into_owned();
        if index % 50 == 0 {
            println!("[sweep] {}/{} {}", index, files.len(), file);
        }

        // Resolve names via an independently-parsed atom table over the same
        // bytes: `AtomTable::with_common_atoms()` + `load_beam_chunks` is the
        // exact sequence the AOT path uses, so the interning order — and hence
        // the atom indices — match. Failure to resolve falls back to the raw
        // atom debug form; the repo is never modified to widen the surface.
        let names = resolve_export_names(&found.path);

        *LAST_PANIC.lock().expect("panic slot") = None;
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            // Fresh compiler per module: bounds JIT code memory across the
            // sweep and isolates a Cranelift panic to one module.
            let compiler = AotCompiler::new()?;
            let result = compiler.compile_module(&found.path)?;
            let compiled: Vec<(Atom, u8)> = result
                .compiled_functions()
                .iter()
                .map(|(atom, arity, _)| (*atom, *arity))
                .collect();
            let skipped: Vec<(Atom, u8, String)> = result.skipped_functions().to_vec();
            Ok::<_, AotError>((compiled, skipped))
        }));

        match outcome {
            Ok(Ok((compiled, skipped))) => {
                for (atom, arity) in &compiled {
                    export_rows.push(ExportRow {
                        corpus: found.corpus,
                        app: found.app.clone(),
                        file: file.clone(),
                        function: name_of(&names, *atom),
                        arity: *arity,
                        outcome: "COMPILED",
                        raw_reason: String::new(),
                    });
                }
                for (atom, arity, reason) in &skipped {
                    export_rows.push(ExportRow {
                        corpus: found.corpus,
                        app: found.app.clone(),
                        file: file.clone(),
                        function: name_of(&names, *atom),
                        arity: *arity,
                        outcome: "SKIPPED",
                        raw_reason: reason.clone(),
                    });
                }
                module_rows.push(ModuleRow {
                    corpus: found.corpus,
                    app: found.app.clone(),
                    file,
                    module_outcome: "loaded".to_owned(),
                    n_exports: compiled.len() + skipped.len(),
                    n_compiled: compiled.len(),
                    n_skipped: skipped.len(),
                });
            }
            Ok(Err(error)) => {
                let reason = flatten(&error.to_string());
                let module_outcome = match &error {
                    AotError::Jit(_) => format!("module-fatal-jit-error({reason})"),
                    _ => format!("load-error({reason})"),
                };
                module_rows.push(ModuleRow {
                    corpus: found.corpus,
                    app: found.app.clone(),
                    file,
                    module_outcome,
                    n_exports: 0,
                    n_compiled: 0,
                    n_skipped: 0,
                });
            }
            Err(payload) => {
                let recorded = LAST_PANIC.lock().expect("panic slot").clone();
                let text = recorded.unwrap_or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
                });
                println!("[PANIC] {file}: {text}");
                module_rows.push(ModuleRow {
                    corpus: found.corpus,
                    app: found.app.clone(),
                    file,
                    module_outcome: format!("module-fatal-jit-error(PANIC: {})", flatten(&text)),
                    n_exports: 0,
                    n_compiled: 0,
                    n_skipped: 0,
                });
            }
        }
    }
}

fn resolve_export_names(path: &Path) -> HashMap<Atom, String> {
    let mut map = HashMap::new();
    let Ok(bytes) = std::fs::read(path) else {
        return map;
    };
    let table = AtomTable::with_common_atoms();
    let Ok(parsed) = load_beam_chunks(&bytes, &table) else {
        return map;
    };
    for export in &parsed.exports {
        if let Some(name) = table.resolve(export.function) {
            map.insert(export.function, name.to_owned());
        }
    }
    map
}

fn name_of(names: &HashMap<Atom, String>, atom: Atom) -> String {
    names
        .get(&atom)
        .cloned()
        .unwrap_or_else(|| format!("<unresolved {atom:?}>"))
}

// -------------------------------------------------------------- normalizing

/// Collapses numeric specifics (label ids, indices, atom numbers) to `N` so
/// identical mechanisms group. A digit run preceded by a letter is left alone
/// so identifiers like `utf8` / `int64` are not mangled.
fn normalize(reason: &str) -> String {
    let chars: Vec<char> = reason.chars().collect();
    let mut out = String::with_capacity(reason.len());
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch.is_ascii_digit() {
            let preceded_by_letter = index > 0 && chars[index - 1].is_alphabetic();
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if preceded_by_letter {
                out.extend(&chars[start..index]);
            } else {
                out.push('N');
            }
        } else if ch.is_whitespace() {
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            out.push(' ');
        } else {
            out.push(ch);
            index += 1;
        }
    }
    out.trim().to_owned()
}

/// Second-order collapse: the Debug form of an instruction embeds its whole
/// operand list, so one mechanism (e.g. `SelectTupleArity`) fragments into a
/// row per operand-list length. This keeps the constructor head and any
/// trailing explanatory clause, and elides the operand payload — unless the
/// payload is a bare tag (`TypeTest(IsNil)`, `Bif(Bif1)`), which IS the
/// mechanism and must survive.
fn mechanism(normalized: &str) -> String {
    let chars: Vec<char> = normalized.chars().collect();
    let Some(open) = chars.iter().position(|c| *c == '{' || *c == '(') else {
        return normalized.to_owned();
    };
    let opener = chars[open];
    let closer = if opener == '{' { '}' } else { ')' };
    let mut depth = 0usize;
    let mut close = None;
    for (index, ch) in chars.iter().enumerate().skip(open) {
        if *ch == opener {
            depth += 1;
        } else if *ch == closer {
            depth -= 1;
            if depth == 0 {
                close = Some(index);
                break;
            }
        }
    }
    let Some(close) = close else {
        return normalized.to_owned();
    };
    let head: String = chars[..open].iter().collect();
    let inner: String = chars[open + 1..close].iter().collect();
    let tail: String = chars[close + 1..].iter().collect();
    let bare_tag = opener == '('
        && !inner.is_empty()
        && !inner.contains(['{', '(', ',', '[']);
    let body = if bare_tag {
        format!("({inner})")
    } else {
        format!("{opener}…{closer}")
    };
    format!("{head}{body}{tail}")
}

fn flatten(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_owned()
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let head: String = text.chars().take(limit).collect();
        format!("{head}…")
    }
}

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

// ------------------------------------------------------------------ outputs

fn write_raw_tsv(path: &Path, rows: &[ExportRow]) {
    let mut out = String::from("corpus\tapp\tfile\tfunction\tarity\toutcome\traw_reason\n");
    for row in rows {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.corpus,
            flatten(&row.app),
            flatten(&row.file),
            flatten(&row.function),
            row.arity,
            row.outcome,
            flatten(&row.raw_reason)
        );
    }
    std::fs::write(path, out).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("[write] {} ({} export rows)", path.display(), rows.len());
}

fn write_module_tsv(path: &Path, rows: &[ModuleRow]) {
    let mut out =
        String::from("corpus\tapp\tfile\tmodule_outcome\tn_exports\tn_compiled\tn_skipped\n");
    for row in rows {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.corpus,
            flatten(&row.app),
            flatten(&row.file),
            flatten(&row.module_outcome),
            row.n_exports,
            row.n_compiled,
            row.n_skipped
        );
    }
    std::fs::write(path, out).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("[write] {} ({} module rows)", path.display(), rows.len());
}

#[derive(Default)]
struct ReasonStat {
    functions_total: usize,
    functions_fixtures: usize,
    functions_otp: usize,
    modules: HashSet<String>,
    modules_fixtures: HashSet<String>,
    modules_otp: HashSet<String>,
    examples: Vec<String>,
}

fn write_ranking(
    path: &Path,
    export_rows: &[ExportRow],
    module_rows: &[ModuleRow],
    files_found: usize,
    anomalies: &[String],
) {
    let corpora = ["FIXTURES", "OTP_STDLIB"];

    // --- module-level tallies ---
    let mut mod_loaded = HashMap::<&str, usize>::new();
    let mut mod_load_error = HashMap::<&str, usize>::new();
    let mut mod_fatal = HashMap::<&str, usize>::new();
    let mut load_error_reasons: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    let mut fatal_reasons: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    let mut fatal_examples: Vec<String> = Vec::new();

    for row in module_rows {
        if row.module_outcome == "loaded" {
            *mod_loaded.entry(row.corpus).or_default() += 1;
        } else if let Some(rest) = row.module_outcome.strip_prefix("load-error(") {
            *mod_load_error.entry(row.corpus).or_default() += 1;
            let raw = rest.trim_end_matches(')').to_owned();
            // NOT normalized: in a loader finding the number IS the finding
            // (opcode 185 and opcode 186 are different coverage gaps).
            let entry = load_error_reasons.entry(raw).or_default();
            entry.0 += 1;
            let basename = row.file.rsplit('/').next().unwrap_or(&row.file).to_owned();
            entry.1.push(format!("{}/{basename}", row.app));
        } else {
            *mod_fatal.entry(row.corpus).or_default() += 1;
            let raw = row
                .module_outcome
                .strip_prefix("module-fatal-jit-error(")
                .unwrap_or(&row.module_outcome)
                .trim_end_matches(')')
                .to_owned();
            let entry = fatal_reasons.entry(normalize(&raw)).or_default();
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(raw.clone());
            }
            if fatal_examples.len() < 25 {
                fatal_examples.push(format!("{} — {}", row.file, clip(&raw, 160)));
            }
        }
    }

    // --- export-level tallies ---
    let mut exports_total = HashMap::<&str, usize>::new();
    let mut compiled = HashMap::<&str, usize>::new();
    let mut skipped = HashMap::<&str, usize>::new();
    let mut stats: HashMap<String, ReasonStat> = HashMap::new();

    for row in export_rows {
        *exports_total.entry(row.corpus).or_default() += 1;
        match row.outcome {
            "COMPILED" => *compiled.entry(row.corpus).or_default() += 1,
            _ => {
                *skipped.entry(row.corpus).or_default() += 1;
                let key = normalize(&row.raw_reason);
                let stat = stats.entry(key).or_default();
                stat.functions_total += 1;
                stat.modules.insert(row.file.clone());
                if row.corpus == "FIXTURES" {
                    stat.functions_fixtures += 1;
                    stat.modules_fixtures.insert(row.file.clone());
                } else {
                    stat.functions_otp += 1;
                    stat.modules_otp.insert(row.file.clone());
                }
                let example = flatten(&row.raw_reason);
                if stat.examples.len() < 3 && !stat.examples.contains(&example) {
                    stat.examples.push(example);
                }
            }
        }
    }

    let mut ranked: Vec<(&String, &ReasonStat)> = stats.iter().collect();
    ranked.sort_by(|a, b| {
        b.1.functions_total
            .cmp(&a.1.functions_total)
            .then_with(|| b.1.modules.len().cmp(&a.1.modules.len()))
            .then_with(|| a.0.cmp(b.0))
    });

    let mut md = String::new();
    let _ = writeln!(md, "# JIT admission rejection census");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "REAL-ERLC-ADMISSION arc, §4 Leg 2. Every exported function of every module below \
         was pushed through `beamr::jit::aot::AotCompiler::compile_module` — the same slicer, \
         pre-pass and Cranelift lowering the demand path uses."
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "- FIXTURES root: `{FIXTURE_ROOT}`");
    let _ = writeln!(md, "- OTP_STDLIB root: `{OTP_LIB_ROOT}/*/ebin/*.beam` (all applications)");
    let _ = writeln!(
        md,
        "- Normalization: digit runs collapse to `N` unless preceded by a letter \
         (so `utf8`/`int64` survive); whitespace runs collapse to one space."
    );
    let _ = writeln!(md);

    if anomalies.is_empty() {
        let _ = writeln!(md, "**Self-tests:** positive control PASS, negative control PASS.");
    } else {
        let _ = writeln!(md, "**SELF-TEST ANOMALIES:**");
        for line in anomalies {
            let _ = writeln!(md, "- {line}");
        }
    }
    let _ = writeln!(md);

    // --- headline ---
    let _ = writeln!(md, "## 1. Headline");
    let _ = writeln!(md);
    let _ = writeln!(md, "Total `.beam` files found: **{files_found}**; module rows written: **{}** (conservation asserted equal).", module_rows.len());
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| corpus | modules | loaded | load-error | module-fatal | exports | compiled | skipped | compiled % |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut all = [0usize; 7];
    for corpus in corpora {
        let loaded = *mod_loaded.get(corpus).unwrap_or(&0);
        let lerr = *mod_load_error.get(corpus).unwrap_or(&0);
        let fatal = *mod_fatal.get(corpus).unwrap_or(&0);
        let exports = *exports_total.get(corpus).unwrap_or(&0);
        let comp = *compiled.get(corpus).unwrap_or(&0);
        let skip = *skipped.get(corpus).unwrap_or(&0);
        let modules = loaded + lerr + fatal;
        let pct = if exports == 0 {
            0.0
        } else {
            100.0 * comp as f64 / exports as f64
        };
        let _ = writeln!(
            md,
            "| {corpus} | {modules} | {loaded} | {lerr} | {fatal} | {exports} | {comp} | {skip} | {pct:.2}% |"
        );
        for (slot, value) in all
            .iter_mut()
            .zip([modules, loaded, lerr, fatal, exports, comp, skip])
        {
            *slot += value;
        }
    }
    let pct = if all[4] == 0 {
        0.0
    } else {
        100.0 * all[5] as f64 / all[4] as f64
    };
    let _ = writeln!(
        md,
        "| **COMBINED** | {} | {} | {} | {} | {} | {} | {} | {pct:.2}% |",
        all[0], all[1], all[2], all[3], all[4], all[5], all[6]
    );
    let _ = writeln!(md);

    // --- mechanism-level ranking (primary) ---
    let mut mech: HashMap<String, ReasonStat> = HashMap::new();
    for row in export_rows {
        if row.outcome == "COMPILED" {
            continue;
        }
        let key = mechanism(&normalize(&row.raw_reason));
        let stat = mech.entry(key).or_default();
        stat.functions_total += 1;
        stat.modules.insert(row.file.clone());
        if row.corpus == "FIXTURES" {
            stat.functions_fixtures += 1;
            stat.modules_fixtures.insert(row.file.clone());
        } else {
            stat.functions_otp += 1;
            stat.modules_otp.insert(row.file.clone());
        }
        let example = flatten(&row.raw_reason);
        if stat.examples.len() < 3 && !stat.examples.contains(&example) {
            stat.examples.push(example);
        }
    }
    let mut mech_ranked: Vec<(&String, &ReasonStat)> = mech.iter().collect();
    mech_ranked.sort_by(|a, b| {
        b.1.functions_total
            .cmp(&a.1.functions_total)
            .then_with(|| b.1.modules.len().cmp(&a.1.modules.len()))
            .then_with(|| a.0.cmp(b.0))
    });

    let _ = writeln!(md, "## 2. Rejection ranking, MECHANISM level (the dispatch list)");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "The `Debug` form of an instruction embeds its whole operand list, so a single \
         mechanism (e.g. `SelectTupleArity`) fragments into one row per operand-list length \
         under plain normalization. This table elides the operand payload — but keeps a bare \
         tag payload (`TypeTest(IsNil)`, `Bif(Bif1)`), which IS the mechanism. \
         Distinct mechanisms: **{}**.",
        mech_ranked.len()
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| # | mechanism | fns (all) | mods (all) | fns FIXTURES | mods FIXTURES | fns OTP | mods OTP | % of all skips | cumulative % |"
    );
    let _ = writeln!(md, "|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    let total_skips_all = all[6].max(1);
    let mut cumulative = 0usize;
    for (rank, (key, stat)) in mech_ranked.iter().enumerate() {
        cumulative += stat.functions_total;
        let _ = writeln!(
            md,
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {:.2}% | {:.2}% |",
            rank + 1,
            md_cell(&clip(key, 220)),
            stat.functions_total,
            stat.modules.len(),
            stat.functions_fixtures,
            stat.modules_fixtures.len(),
            stat.functions_otp,
            stat.modules_otp.len(),
            100.0 * stat.functions_total as f64 / total_skips_all as f64,
            100.0 * cumulative as f64 / total_skips_all as f64
        );
    }
    let _ = writeln!(md);
    let _ = writeln!(md, "### 2b. Mechanism → verbatim examples");
    let _ = writeln!(md);
    let _ = writeln!(md, "| # | mechanism | verbatim examples (up to 3) |");
    let _ = writeln!(md, "|---:|---|---|");
    for (rank, (key, stat)) in mech_ranked.iter().enumerate() {
        let examples = stat
            .examples
            .iter()
            .map(|e| format!("`{}`", md_cell(&clip(e, 300))))
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = writeln!(
            md,
            "| {} | `{}` | {examples} |",
            rank + 1,
            md_cell(&clip(key, 220))
        );
    }
    let _ = writeln!(md);

    // --- ranking ---
    let _ = writeln!(md, "## 3. Rejection ranking, EXACT normalized reason");
    let _ = writeln!(md);
    let _ = writeln!(md, "Distinct normalized rejection reasons: **{}**.", ranked.len());
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| # | normalized reason | fns (all) | mods (all) | fns FIXTURES | mods FIXTURES | fns OTP | mods OTP | % of all skips |"
    );
    let _ = writeln!(md, "|---:|---|---:|---:|---:|---:|---:|---:|---:|");
    let total_skips = all[6].max(1);
    for (rank, (key, stat)) in ranked.iter().enumerate() {
        let _ = writeln!(
            md,
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {:.2}% |",
            rank + 1,
            md_cell(&clip(key, 220)),
            stat.functions_total,
            stat.modules.len(),
            stat.functions_fixtures,
            stat.modules_fixtures.len(),
            stat.functions_otp,
            stat.modules_otp.len(),
            100.0 * stat.functions_total as f64 / total_skips as f64
        );
    }
    let _ = writeln!(md);

    // --- verbatim examples ---
    let _ = writeln!(md, "## 4. Exact normalized reason → verbatim examples");
    let _ = writeln!(md);
    let _ = writeln!(md, "| # | normalized reason | verbatim examples (up to 3) |");
    let _ = writeln!(md, "|---:|---|---|");
    for (rank, (key, stat)) in ranked.iter().enumerate() {
        let examples = stat
            .examples
            .iter()
            .map(|e| format!("`{}`", md_cell(&clip(e, 300))))
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = writeln!(
            md,
            "| {} | `{}` | {examples} |",
            rank + 1,
            md_cell(&clip(key, 220))
        );
    }
    let _ = writeln!(md);

    // --- loader findings, kept separate ---
    let _ = writeln!(md, "## 5. Loader coverage findings (NOT JIT rejections)");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "These modules never reached the JIT: `load_beam_chunks` (or the file read) refused them. \
         They are a LOADER coverage census and must not be mixed into the ranking above."
    );
    let _ = writeln!(md);
    let mut load_ranked: Vec<(&String, &(usize, Vec<String>))> = load_error_reasons.iter().collect();
    load_ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
    let _ = writeln!(
        md,
        "Reasons here are VERBATIM, not normalized: the numeric specific is the finding \
         (opcode 185 and opcode 186 are distinct loader coverage gaps)."
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "| # | verbatim load-error | modules | example modules |");
    let _ = writeln!(md, "|---:|---|---:|---|");
    for (rank, (key, (count, examples))) in load_ranked.iter().enumerate() {
        let ex = examples
            .iter()
            .map(|e| format!("`{}`", md_cell(&clip(e, 200))))
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = writeln!(
            md,
            "| {} | `{}` | {count} | {ex} |",
            rank + 1,
            md_cell(&clip(key, 220))
        );
    }
    let _ = writeln!(md);

    // --- module-fatal ---
    let _ = writeln!(md, "## 6. Module-fatal JIT errors");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "A non-skippable `JitError` (or a panic) aborts the whole module in `compile_module`, \
         so its remaining exports are never classified. Count: **{}**.",
        all[3]
    );
    let _ = writeln!(md);
    let mut fatal_ranked: Vec<(&String, &(usize, Vec<String>))> = fatal_reasons.iter().collect();
    fatal_ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
    if fatal_ranked.is_empty() {
        let _ = writeln!(md, "None.");
    } else {
        let _ = writeln!(md, "| # | normalized fatal reason | modules | verbatim examples |");
        let _ = writeln!(md, "|---:|---|---:|---|");
        for (rank, (key, (count, examples))) in fatal_ranked.iter().enumerate() {
            let ex = examples
                .iter()
                .map(|e| format!("`{}`", md_cell(&clip(e, 200))))
                .collect::<Vec<_>>()
                .join("<br>");
            let _ = writeln!(
                md,
                "| {} | `{}` | {count} | {ex} |",
                rank + 1,
                md_cell(&clip(key, 220))
            );
        }
        let _ = writeln!(md);
        let _ = writeln!(md, "First module-fatal modules (up to 25):");
        let _ = writeln!(md);
        for line in &fatal_examples {
            let _ = writeln!(md, "- `{}`", md_cell(line));
        }
    }
    let _ = writeln!(md);

    // --- per-app ---
    let _ = writeln!(md, "## 7. Per-application split (OTP_STDLIB)");
    let _ = writeln!(md);
    let mut per_app: BTreeMap<&str, [usize; 6]> = BTreeMap::new();
    for row in module_rows {
        if row.corpus != "OTP_STDLIB" {
            continue;
        }
        let slot = per_app.entry(row.app.as_str()).or_default();
        slot[0] += 1;
        if row.module_outcome == "loaded" {
            slot[1] += 1;
        } else if row.module_outcome.starts_with("load-error(") {
            slot[2] += 1;
        } else {
            slot[5] += 1;
        }
        slot[3] += row.n_compiled;
        slot[4] += row.n_skipped;
    }
    let _ = writeln!(
        md,
        "| app | modules | loaded | load-error | module-fatal | compiled | skipped |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|");
    for (app, slot) in &per_app {
        let _ = writeln!(
            md,
            "| {app} | {} | {} | {} | {} | {} | {} |",
            slot[0], slot[1], slot[2], slot[5], slot[3], slot[4]
        );
    }

    std::fs::write(path, md).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("[write] {}", path.display());
}
