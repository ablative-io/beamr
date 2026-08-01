//! P4 probe — what wiring the virtual-binary-heap increment does to GC pacing.
//!
//! Two arms over ONE deterministic binary-heavy workload. The single variable
//! is whether `Process::increase_virtual_binary_heap` is called when a ProcBin
//! is placed on a process heap:
//!
//!   arm A  as-is    — no increment (what production does at e168115)
//!   arm B  trigger-live — increment by the ProcBin's off-heap byte count
//!
//! ARM B IS SEAM-NEUTRAL BY CONSTRUCTION. It does not patch any production
//! allocation site: the increment is made from this probe, at the probe's own
//! ProcBin allocation, through the already-public
//! `Process::increase_virtual_binary_heap` (`process/mod.rs:396`). The probe
//! therefore makes NO choice about which production seam a future wiring brief
//! should use — see the artifact README's seam-candidate list.
//!
//! No wall-clock, no RNG: every number here is a count or a word/byte total.
//!
//! GC-invocation counting uses no new production instrumentation. It is
//! DERIVED, exactly, from the pre-call process state, per `gc/mod.rs:135-176`:
//!
//!   alloc(p, w):     if !pressure(p) { if young.available() >= w -> return }   // no GC
//!                    ensure_space(p, w, 256)
//!   ensure_space:    if available() >= w && !pressure(p) { return }            // no GC
//!                    collect_minor_with_live(...)                             // GC
//!
//! Case analysis over (pressure, available() >= w) shows a collection runs iff
//! `pressure || available() < w`. That predicate is computed below from public
//! accessors only. A second, independent signal (young-generation residency
//! not advancing by exactly the allocation) corroborates it.

use std::mem::size_of;

use beamr::gc;
use beamr::process::Process;
use beamr::process::heap::DEFAULT_HEAP_SIZE;
use beamr::term::Term;
use beamr::term::shared_binary::{SharedBinary, write_proc_bin};

/// Mirrors the private `WORD_BYTES` at `gc/mod.rs:14`.
const WORD_BYTES: usize = size_of::<u64>();

/// ProcBin layout is header + flags + Arc pointer — `term/shared_binary.rs:20-21`.
const PROC_BIN_WORDS: usize = 3;

/// Byte sizes for the workload. All are strictly above
/// `REFC_BINARY_THRESHOLD` (64, `term/shared_binary.rs:18`), so each one lands
/// as a refcounted ProcBin rather than an inline heap binary.
const SIZES: [usize; 4] = [65, 512, 4096, 65536];

/// (label, initial heap words). The default-heap process is the production
/// shape; the large-heap process controls for "this is only an artefact of the
/// 233-word default nursery".
const HEAP_CONFIGS: [(&str, usize); 2] =
    [("default-233", DEFAULT_HEAP_SIZE), ("large-32768", 32_768)];

const PROCESSES_PER_CONFIG: u64 = 4;
const ALLOCS_PER_PROCESS: usize = 256;
/// One in four allocated binaries is retained as a live root; the rest are
/// dropped (become unreachable) — "created and dropped across N processes".
const RETAIN_EVERY: usize = 4;
/// Rolling window of live roots held in x registers.
const ROOT_SLOTS: u16 = 16;

#[derive(Clone, Copy)]
struct Arm {
    name: &'static str,
    wire_increment: bool,
}

#[derive(Default, Clone, Copy)]
struct Counters {
    allocations: u64,
    alloc_failures: u64,
    /// Collections implied by the pre-call state (exact; see module docs).
    derived_collections: u64,
    /// Of those, the ones forced by the vheap-pressure predicate alone.
    forced_by_pressure_only: u64,
    /// Of those, the ones that would have happened anyway (nursery too small).
    forced_by_no_room: u64,
    /// Independent corroboration: young residency did not advance by exactly
    /// the allocation, or a region capacity changed.
    observed_collections: u64,
}

struct ProcessEnd {
    config: &'static str,
    pid: u64,
    total_used_words: usize,
    total_capacity_words: usize,
    young_used_words: usize,
    old_used_words: usize,
    young_capacity_words: usize,
    old_capacity_words: usize,
    virtual_binary_heap_bytes: usize,
}

/// Re-implementation of the private `virtual_binary_pressure_exceeds_heap`
/// (`gc/mod.rs:172-176`), byte-for-byte in structure, over public accessors.
fn vheap_pressure_exceeds_heap(process: &Process) -> bool {
    let heap_used_bytes = process.heap().total_used().saturating_mul(WORD_BYTES);
    let heap_capacity_bytes = process.heap().total_capacity().saturating_mul(WORD_BYTES);
    heap_used_bytes.saturating_add(process.virtual_binary_heap()) >= heap_capacity_bytes
}

/// Place one ProcBin on `process`'s heap the way production does — GC-aware
/// allocation, then mark the allocation so the release walk sees it, then
/// write the ProcBin layout. Compare `interpreter/opcodes/binary/matching.rs:626-635`
/// (production, no increment) and `gc/tests.rs:78-87` (test helper, increment).
fn alloc_proc_bin(
    process: &mut Process,
    shared: &SharedBinary,
    wire_increment: bool,
) -> Option<Term> {
    let ptr = match gc::alloc(process, PROC_BIN_WORDS) {
        Ok(ptr) => ptr,
        Err(_gc_error) => return None,
    };
    // SAFETY: `gc::alloc` returned PROC_BIN_WORDS writable, contiguous words on
    // this process's young generation; the slice is used immediately to
    // initialise that allocation and is not held across any further allocation.
    let words = unsafe { std::slice::from_raw_parts_mut(ptr, PROC_BIN_WORDS) };
    let term = write_proc_bin(words, shared)?;
    process
        .heap_mut()
        .mark_last_young_allocation_maybe_refcounted();
    if wire_increment {
        process.increase_virtual_binary_heap(shared.len());
    }
    Some(term)
}

fn run_process(
    arm: Arm,
    config: &'static str,
    heap_words: usize,
    pid: u64,
) -> (Counters, ProcessEnd) {
    let mut counters = Counters::default();
    let mut process = Process::new(pid, heap_words);
    let mut retained: u16 = 0;

    for index in 0..ALLOCS_PER_PROCESS {
        let size = SIZES[index % SIZES.len()];
        let shared = SharedBinary::new(vec![0xA5_u8; size]);

        let pressure = vheap_pressure_exceeds_heap(&process);
        let has_room = process.heap().available() >= PROC_BIN_WORDS;
        if pressure || !has_room {
            counters.derived_collections += 1;
            if pressure && has_room {
                counters.forced_by_pressure_only += 1;
            } else {
                counters.forced_by_no_room += 1;
            }
        }

        let young_used_before = process.heap().young_used();
        let old_used_before = process.heap().old_used();
        let young_capacity_before = process.heap().young_capacity();
        let old_capacity_before = process.heap().old_capacity();

        counters.allocations += 1;
        match alloc_proc_bin(&mut process, &shared, arm.wire_increment) {
            Some(term) => {
                if index % RETAIN_EVERY == 0 {
                    process.set_x_reg(retained % ROOT_SLOTS, term);
                    retained = retained.wrapping_add(1);
                }
            }
            None => counters.alloc_failures += 1,
        }

        let advanced_exactly = process.heap().young_used()
            == young_used_before.saturating_add(PROC_BIN_WORDS)
            && process.heap().old_used() == old_used_before
            && process.heap().young_capacity() == young_capacity_before
            && process.heap().old_capacity() == old_capacity_before;
        if !advanced_exactly {
            counters.observed_collections += 1;
        }
    }

    let end = ProcessEnd {
        config,
        pid,
        total_used_words: process.heap().total_used(),
        total_capacity_words: process.heap().total_capacity(),
        young_used_words: process.heap().young_used(),
        old_used_words: process.heap().old_used(),
        young_capacity_words: process.heap().young_capacity(),
        old_capacity_words: process.heap().old_capacity(),
        virtual_binary_heap_bytes: process.virtual_binary_heap(),
    };
    (counters, end)
}

fn accumulate(totals: &mut Counters, counters: Counters) {
    totals.allocations += counters.allocations;
    totals.alloc_failures += counters.alloc_failures;
    totals.derived_collections += counters.derived_collections;
    totals.forced_by_pressure_only += counters.forced_by_pressure_only;
    totals.forced_by_no_room += counters.forced_by_no_room;
    totals.observed_collections += counters.observed_collections;
}

fn run_arm(arm: Arm) -> (Counters, Vec<(&'static str, Counters)>, Vec<ProcessEnd>) {
    let mut totals = Counters::default();
    let mut per_config = Vec::new();
    let mut ends = Vec::new();
    let mut pid = 1_u64;

    for (config, heap_words) in HEAP_CONFIGS {
        let mut config_totals = Counters::default();
        for _ in 0..PROCESSES_PER_CONFIG {
            let (counters, end) = run_process(arm, config, heap_words, pid);
            accumulate(&mut config_totals, counters);
            accumulate(&mut totals, counters);
            ends.push(end);
            pid += 1;
        }
        per_config.push((config, config_totals));
    }

    (totals, per_config, ends)
}

fn main() {
    println!("# P4 vheap/GC-pacing probe — beamr lane #62");
    println!("# tree: e16811597c3c1bde75f0e94c204d0497be8a7e05");
    println!("# WORD_BYTES={WORD_BYTES} PROC_BIN_WORDS={PROC_BIN_WORDS}");
    println!("# REFC threshold is 64 bytes; every workload size below exceeds it.");
    println!("# sizes={SIZES:?} allocs_per_process={ALLOCS_PER_PROCESS}");
    println!("# processes_per_config={PROCESSES_PER_CONFIG} retain_every={RETAIN_EVERY}");
    println!("# heap_configs={HEAP_CONFIGS:?}");
    println!();

    let arms = [
        Arm {
            name: "A-as-is",
            wire_increment: false,
        },
        Arm {
            name: "B-trigger-live",
            wire_increment: true,
        },
    ];

    let mut summaries = Vec::new();

    for arm in arms {
        let (totals, per_config, ends) = run_arm(arm);

        println!(
            "## ARM {} (wire_increment={})",
            arm.name, arm.wire_increment
        );
        println!(
            "arm\tconfig\tpid\ttotal_used_w\ttotal_cap_w\tyoung_used_w\told_used_w\tyoung_cap_w\told_cap_w\tvheap_bytes"
        );
        for end in &ends {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                arm.name,
                end.config,
                end.pid,
                end.total_used_words,
                end.total_capacity_words,
                end.young_used_words,
                end.old_used_words,
                end.young_capacity_words,
                end.old_capacity_words,
                end.virtual_binary_heap_bytes
            );
        }

        let residency_words: usize = ends.iter().map(|end| end.total_used_words).sum();
        let capacity_words: usize = ends.iter().map(|end| end.total_capacity_words).sum();
        // `erlang:memory(binary)` derivation. `scheduler/mod.rs:852-875` sums,
        // over every non-exited process slot, either the live
        // `virtual_binary_heap()` (Present) or `metadata.binary_heap_size`
        // (Executing) — and the sole production writer of the latter,
        // `scheduler/execution/core.rs:321`, writes `virtual_binary_heap()`.
        // Both slot states therefore reduce to this sum.
        let memory_binary: usize = ends.iter().map(|end| end.virtual_binary_heap_bytes).sum();

        println!();
        println!(
            "arm\tconfig\tallocations\tgc_derived\tby_vheap_pressure_only\tby_no_nursery_room\tgc_observed"
        );
        for (config, counters) in &per_config {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                arm.name,
                config,
                counters.allocations,
                counters.derived_collections,
                counters.forced_by_pressure_only,
                counters.forced_by_no_room,
                counters.observed_collections
            );
        }

        println!();
        println!("arm={}", arm.name);
        println!("allocations={}", totals.allocations);
        println!("alloc_failures={}", totals.alloc_failures);
        println!("gc_collections_derived={}", totals.derived_collections);
        println!(
            "gc_collections_forced_by_vheap_pressure_only={}",
            totals.forced_by_pressure_only
        );
        println!(
            "gc_collections_forced_by_no_nursery_room={}",
            totals.forced_by_no_room
        );
        println!(
            "gc_collections_observed_corroboration={}",
            totals.observed_collections
        );
        println!("end_state_residency_words={residency_words}");
        println!("end_state_capacity_words={capacity_words}");
        println!("erlang_memory_binary_derived_bytes={memory_binary}");
        println!();

        summaries.push((
            arm.name,
            totals,
            residency_words,
            capacity_words,
            memory_binary,
        ));
    }

    println!("## A/B TABLE");
    println!(
        "metric\t{}\t{}",
        summaries.first().map_or("A", |summary| summary.0),
        summaries.get(1).map_or("B", |summary| summary.0)
    );
    if let (Some(first), Some(second)) = (summaries.first(), summaries.get(1)) {
        println!(
            "allocations\t{}\t{}",
            first.1.allocations, second.1.allocations
        );
        println!(
            "alloc_failures\t{}\t{}",
            first.1.alloc_failures, second.1.alloc_failures
        );
        println!(
            "gc_collections_derived\t{}\t{}",
            first.1.derived_collections, second.1.derived_collections
        );
        println!(
            "gc_collections_forced_by_vheap_pressure_only\t{}\t{}",
            first.1.forced_by_pressure_only, second.1.forced_by_pressure_only
        );
        println!(
            "gc_collections_forced_by_no_nursery_room\t{}\t{}",
            first.1.forced_by_no_room, second.1.forced_by_no_room
        );
        println!(
            "gc_collections_observed_corroboration\t{}\t{}",
            first.1.observed_collections, second.1.observed_collections
        );
        println!("end_state_residency_words\t{}\t{}", first.2, second.2);
        println!("end_state_capacity_words\t{}\t{}", first.3, second.3);
        println!(
            "erlang_memory_binary_derived_bytes\t{}\t{}",
            first.4, second.4
        );
    }
}
