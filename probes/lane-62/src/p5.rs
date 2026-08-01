//! P5 probe — ETS big-binary functional ceiling on a default-heap process.
//!
//! `ets:insert/2` then `ets:lookup/2` at 64 KiB / 512 KiB / 1 MiB / 4 MiB, on a
//! process created with the default heap (`DEFAULT_HEAP_SIZE`, so
//! `max_capacity` is `DEFAULT_MAX_HEAP_WORDS` = 131_072 words). The claim under
//! test is that the documented "memory cost" is, at the bytes, a FUNCTIONAL
//! CEILING: the caller gets `badarg`, indistinguishable at the BIF boundary
//! from a genuinely bad argument.
//!
//! The probe also bisects the exact crossover byte size, so the ~1 MiB figure
//! stops being arithmetic on a constant and becomes an observation.
//!
//! Mechanism being observed (read at e168115, not assumed):
//!   * `ets:insert` deep-copies into ETS-owned memory; `ets/copy.rs:234-241`
//!     materialises every binary INLINE (`2 + packed_word_count(len)` words) —
//!     a ProcBin argument does not stay refcounted once it is in the table.
//!   * `ets:lookup` -> `copy_rows_list` (`native/ets_bifs.rs:1288-1298`)
//!     reserves `sum(row.total_words()) + 2*rows` up front via
//!     `ProcessContext::ensure_heap_space`.
//!   * `ensure_heap_space` (`native/context/mod.rs:788-795`) maps EVERY
//!     `gc::ensure_space` error to `Term::atom(BADARG)` with `.map_err(|_| …)`.
//!
//! No wall-clock, no RNG.

use std::sync::Arc;

use beamr::atom::table::AtomTable;
use beamr::ets::{EtsRegistry, copy_term_to_ets};
use beamr::native::ProcessContext;
use beamr::native::ets_bifs::{EtsFacility, bif_insert, bif_lookup, bif_new};
use beamr::process::Process;
use beamr::process::heap::{DEFAULT_HEAP_SIZE, DEFAULT_MAX_HEAP_WORDS};
use beamr::term::Term;

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// The four sizes named by the brief.
const SIZES: [(&str, usize); 4] = [
    ("64KiB", 64 * KIB),
    ("512KiB", 512 * KIB),
    ("1MiB", MIB),
    ("4MiB", 4 * MIB),
];

const KEY: i64 = 1;

struct Outcome {
    insert: Result<(), String>,
    lookup: Result<(), String>,
    ets_owned_words: Option<usize>,
    lookup_reserve_words: Option<usize>,
    direct_ensure_space: Option<Result<(), String>>,
}

/// Render a BIF error term exactly as the caller receives it: the raw `Term`
/// Debug form, the `Atom` it carries, and that atom's interned name.
fn render_error(term: Term, atoms: &AtomTable) -> String {
    match term.as_atom() {
        Some(atom) => {
            let name = atoms.resolve(atom).unwrap_or("<unresolved>");
            format!("Term={term:?} Atom={atom:?} name={name:?}")
        }
        None => format!("Term={term:?} (not an atom)"),
    }
}

fn trial(size: usize) -> Outcome {
    let atoms = Arc::new(AtomTable::with_common_atoms());
    let facility: Arc<dyn EtsFacility> = Arc::new(EtsRegistry::new(Arc::clone(&atoms)));
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_atom_table(Some(Arc::clone(&atoms)));
    context.set_ets_facility(Some(Arc::clone(&facility)));
    context.set_pid(Some(1));
    context.attach_process(&mut process, 0);

    let mut outcome = Outcome {
        insert: Err("not reached".to_owned()),
        lookup: Err("not reached".to_owned()),
        ets_owned_words: None,
        lookup_reserve_words: None,
        direct_ensure_space: None,
    };

    let bytes = vec![0xA5_u8; size];
    let binary = match context.alloc_binary(&bytes) {
        Ok(term) => term,
        Err(term) => {
            outcome.insert = Err(format!("alloc_binary: {}", render_error(term, &atoms)));
            return outcome;
        }
    };
    let key = Term::small_int(KEY);
    let tuple = match context.alloc_tuple(&[key, binary]) {
        Ok(term) => term,
        Err(term) => {
            outcome.insert = Err(format!("alloc_tuple: {}", render_error(term, &atoms)));
            return outcome;
        }
    };

    // Measure the exact ETS-owned word count the row will occupy, by running
    // the same copy `ets:insert` runs. This is the number `copy_rows_list`
    // reserves against on the way back out.
    match copy_term_to_ets(tuple) {
        Ok(owned) => {
            let words = owned.total_words();
            outcome.ets_owned_words = Some(words);
            // `copy_rows_list` adds `list_heap_words(1)` = 2 words for the
            // one-element result list (`native/ets_bifs.rs:1267-1269, 1288-1291`).
            outcome.lookup_reserve_words = Some(words.saturating_add(2));
        }
        Err(error) => {
            outcome.ets_owned_words = None;
            outcome.insert = Err(format!("copy_term_to_ets: {error:?}"));
        }
    }

    let table_name = atoms.intern("p5_probe_table");
    let table = match bif_new(&[Term::atom(table_name), Term::NIL], &mut context) {
        Ok(term) => term,
        Err(term) => {
            outcome.insert = Err(format!("ets:new: {}", render_error(term, &atoms)));
            return outcome;
        }
    };

    outcome.insert = match bif_insert(&[table, tuple], &mut context) {
        Ok(_term) => Ok(()),
        Err(term) => Err(render_error(term, &atoms)),
    };

    outcome.lookup = match bif_lookup(&[table, key], &mut context) {
        Ok(_term) => Ok(()),
        Err(term) => Err(render_error(term, &atoms)),
    };

    outcome
}

/// Independent confirmation that the badarg above is the heap-limit condition:
/// ask `ensure_heap_space` for the same word budget on a fresh default-heap
/// process, with no ETS in the picture at all.
fn direct_ensure(words: usize) -> Result<(), String> {
    let atoms = AtomTable::with_common_atoms();
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    let mut context = ProcessContext::new();
    context.set_pid(Some(1));
    context.attach_process(&mut process, 0);
    match context.ensure_heap_space(words) {
        Ok(()) => Ok(()),
        Err(term) => Err(render_error(term, &atoms)),
    }
}

/// True exactly when a lookup at `size` returns a term rather than an error.
fn lookup_succeeds(size: usize) -> bool {
    trial(size).lookup.is_ok()
}

/// Largest byte size whose lookup still succeeds, by bisection on bytes.
fn bisect_crossover(mut low_ok: usize, mut high_fail: usize) -> (usize, usize) {
    while high_fail.saturating_sub(low_ok) > 1 {
        let midpoint = low_ok + (high_fail - low_ok) / 2;
        if lookup_succeeds(midpoint) {
            low_ok = midpoint;
        } else {
            high_fail = midpoint;
        }
    }
    (low_ok, high_fail)
}

fn describe(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "OK".to_owned(),
        Err(message) => format!("ERR {message}"),
    }
}

fn main() {
    println!("# P5 ETS big-binary probe — beamr lane #62");
    println!("# tree: e16811597c3c1bde75f0e94c204d0497be8a7e05");
    println!("# DEFAULT_HEAP_SIZE={DEFAULT_HEAP_SIZE} words");
    println!("# DEFAULT_MAX_HEAP_WORDS={DEFAULT_MAX_HEAP_WORDS} words");
    println!("# table: ets:new(p5_probe_table, []) => set, protected, keypos 1");
    println!("# row: {{1, Bin}} — key is an immediate small int");
    println!();

    println!("## SIZE TABLE");
    println!(
        "label\tbytes\tets_owned_words\tlookup_reserve_words\tover_max_capacity\tinsert\tlookup"
    );
    let mut direct_probes = Vec::new();
    for (label, size) in SIZES {
        let mut outcome = trial(size);
        if let Some(words) = outcome.lookup_reserve_words {
            outcome.direct_ensure_space = Some(direct_ensure(words));
        }
        let over = outcome
            .lookup_reserve_words
            .is_some_and(|words| words > DEFAULT_MAX_HEAP_WORDS);
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            label,
            size,
            outcome
                .ets_owned_words
                .map_or_else(|| "-".to_owned(), |words| words.to_string()),
            outcome
                .lookup_reserve_words
                .map_or_else(|| "-".to_owned(), |words| words.to_string()),
            over,
            describe(&outcome.insert),
            describe(&outcome.lookup)
        );
        direct_probes.push((label, outcome));
    }

    println!();
    println!("## EXACT ERROR TERMS (verbatim, as the caller receives them)");
    for (label, outcome) in &direct_probes {
        println!("{label} insert -> {}", describe(&outcome.insert));
        println!("{label} lookup -> {}", describe(&outcome.lookup));
        match &outcome.direct_ensure_space {
            Some(result) => println!(
                "{label} direct ensure_heap_space({}) -> {}",
                outcome
                    .lookup_reserve_words
                    .map_or_else(|| "-".to_owned(), |words| words.to_string()),
                describe(result)
            ),
            None => println!("{label} direct ensure_heap_space -> not run"),
        }
    }

    println!();
    println!("## CROSSOVER (bisection on bytes, fresh default-heap process per trial)");
    let low = 512 * KIB;
    let high = MIB;
    println!("bisection_bracket_low_ok_bytes={low}");
    println!("bisection_bracket_high_bytes={high}");
    println!("bracket_low_lookup_succeeds={}", lookup_succeeds(low));
    println!("bracket_high_lookup_succeeds={}", lookup_succeeds(high));
    let (largest_ok, smallest_fail) = bisect_crossover(low, high);
    println!("largest_size_with_successful_lookup_bytes={largest_ok}");
    println!("smallest_size_with_failing_lookup_bytes={smallest_fail}");
    println!(
        "largest_ok_as_mib_fraction={:.6}",
        largest_ok as f64 / MIB as f64
    );
    let ok_trial = trial(largest_ok);
    let fail_trial = trial(smallest_fail);
    println!(
        "at_largest_ok: ets_owned_words={} lookup_reserve_words={} insert={} lookup={}",
        ok_trial
            .ets_owned_words
            .map_or_else(|| "-".to_owned(), |words| words.to_string()),
        ok_trial
            .lookup_reserve_words
            .map_or_else(|| "-".to_owned(), |words| words.to_string()),
        describe(&ok_trial.insert),
        describe(&ok_trial.lookup)
    );
    println!(
        "at_smallest_fail: ets_owned_words={} lookup_reserve_words={} insert={} lookup={}",
        fail_trial
            .ets_owned_words
            .map_or_else(|| "-".to_owned(), |words| words.to_string()),
        fail_trial
            .lookup_reserve_words
            .map_or_else(|| "-".to_owned(), |words| words.to_string()),
        describe(&fail_trial.insert),
        describe(&fail_trial.lookup)
    );
}
