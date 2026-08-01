//! Acceptance tests for the `gleam_erlang_ffi:sleep_forever/0` suspend fix.
//!
//! `sleep_forever/0` is registered with `dirty_kind: None`, so its body runs
//! synchronously inside the caller's slice on a NORMAL scheduler thread. The
//! old body looped on `std::thread::sleep(Duration::MAX)`, which permanently
//! retired that thread. The fix parks the process through the host-await
//! suspension facility instead (BEAM's `receive after infinity`).
//!
//! The two properties below are deliberately kept in SEPARATE tests because
//! they prove different things and can fail independently:
//!
//!   1. `sleep_forever_parks_consume_no_scheduler_thread_of_either_kind` —
//!      a park costs no thread. This also discriminates the fix from the
//!      rejected "just mark it dirty Io" shortcut: an Io-marked BIF would
//!      still block, merely on a dirty worker, and would move the dirty
//!      instruments this test holds flat.
//!   2. `a_parked_process_remains_subject_to_exit_signals` — a park is not a
//!      leak. The process stays killable from outside.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use beamr::atom::Atom;
use beamr::loader::Instruction;
use beamr::loader::decode::compact::Operand;
use beamr::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use beamr::native::gleam_ffi::bif_sleep_forever;
use beamr::native::{Capability, NativeEntry, NativeFn, ProcessContext};
use beamr::process::ExitReason;
use beamr::scheduler::{Scheduler, SchedulerConfig};
use beamr::term::Term;

/// Bumped by the `normal_progress` native, so "other work still ran" is
/// observed at the native itself rather than inferred from an exit code.
static NORMAL_PROGRESS: AtomicUsize = AtomicUsize::new(0);

/// How many processes are parked in `sleep_forever` before progress is
/// demanded. Far more than the single normal scheduler thread configured
/// below: under the old blocking body the FIRST of these retires that thread
/// and nothing afterwards can ever run.
const PARKED_PROCESSES: usize = 32;

/// Generous relative to the work involved — these bounds exist so a
/// regression fails the run instead of hanging it forever.
const PARK_TIMEOUT: Duration = Duration::from_secs(30);

fn module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            // The catch-all IS the filter, not a swallowed case. Index
            // completeness is not load-bearing here: the only caller
            // (`call_native_module`) emits `CallExt` + `Return`, so this index
            // is always empty and no assertion consults it. A future
            // label-bearing variant falling through would change nothing these
            // tests prove. Same shape as `module.rs:607` in production.
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: HashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

fn normal_progress(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    NORMAL_PROGRESS.fetch_add(1, Ordering::AcqRel);
    Ok(Term::small_int(7))
}

fn native_import(function: NativeFn, capability: Capability) -> ResolvedImport {
    ResolvedImport {
        module: Atom::OK,
        function: Atom::OK,
        arity: 0,
        target: ResolvedImportTarget::Native(NativeEntry {
            function,
            // `None` mirrors `sleep_forever`'s real registration in
            // `GLEAM_PROCESS_BIFS`: it dispatches synchronously, in-slice, on
            // a normal scheduler thread. Marking it dirty here would test a
            // BIF that does not exist.
            dirty_kind: None,
            capability,
        }),
    }
}

fn call_native_module(name: Atom, import: ResolvedImport) -> Module {
    let mut built = module(
        name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    built.resolved_imports.push(import);
    built
}

/// Transient dirty-completion threads spawned to date, read from the public
/// service-policy lines — the same surface an embedder would use.
fn completion_spawned_total(scheduler: &Scheduler) -> u64 {
    scheduler
        .service_policies()
        .into_iter()
        .map(|line| line.spawned_total)
        .sum()
}

/// Poll `predicate` until it yields `Some`, or panic after `timeout`.
fn poll_until<T>(timeout: Duration, message: &str, mut predicate: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = predicate() {
            return value;
        }
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `run_until_exit` on a background thread with a hard deadline: a process
/// that never exits must fail the test rather than wedge the suite.
fn run_until_exit_bounded(
    scheduler: &Arc<Scheduler>,
    pid: u64,
) -> (ExitReason, beamr::ets::OwnedTerm) {
    let (sender, completion) = std::sync::mpsc::channel();
    let scheduler_for_wait = Arc::clone(scheduler);
    std::thread::spawn(move || {
        let _sent = sender.send(scheduler_for_wait.run_until_exit(pid));
    });
    completion
        .recv_timeout(PARK_TIMEOUT)
        .unwrap_or_else(|_| panic!("process {pid} never exited"))
}

fn scheduler_with_one_normal_thread(registry: &Arc<ModuleRegistry>) -> Arc<Scheduler> {
    Arc::new(
        Scheduler::new(
            SchedulerConfig {
                // ONE normal thread makes the old defect fatal rather than
                // merely wasteful: a single retired thread is the whole pool.
                thread_count: Some(1),
                dirty_cpu_threads: Some(1),
                dirty_io_threads: Some(1),
                dirty_queue_depth: Some(8),
                ..SchedulerConfig::default()
            },
            Arc::clone(registry),
        )
        .expect("scheduler starts"),
    )
}

/// Wait until `count` further host-await suspension mirrors have been
/// registered, i.e. that many processes have actually reached the park. This
/// observes the park directly instead of sleeping and hoping.
fn wait_for_parks(scheduler: &Arc<Scheduler>, baseline: u64, count: u64) {
    poll_until(
        PARK_TIMEOUT,
        "processes never reached the sleep_forever park",
        || (scheduler.suspension_mirror_registration_count() >= baseline + count).then_some(()),
    );
}

// ---------------------------------------------------------------------------
// (a) A park costs no scheduler thread, normal or dirty.
// ---------------------------------------------------------------------------

#[test]
fn sleep_forever_parks_consume_no_scheduler_thread_of_either_kind() {
    NORMAL_PROGRESS.store(0, Ordering::Release);

    let registry = Arc::new(ModuleRegistry::new());
    let parked_module = registry.insert(call_native_module(
        Atom::OK,
        native_import(bif_sleep_forever, Capability::Clock),
    ));
    let progress_module = registry.insert(call_native_module(
        Atom::ERROR,
        native_import(normal_progress, Capability::Pure),
    ));

    let scheduler = scheduler_with_one_normal_thread(&registry);

    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let dirty_allocations_before = scheduler.dirty_suspension_allocation_count();
    let dirty_completions_before = completion_spawned_total(&scheduler);

    let parked: Vec<u64> = (0..PARKED_PROCESSES)
        .map(|_| scheduler.spawn_process(&parked_module))
        .collect();
    wait_for_parks(&scheduler, mirrors_before, PARKED_PROCESSES as u64);

    // Parked, not finished: a process that merely returned would be gone.
    for pid in &parked {
        assert!(
            scheduler.process_table().get(*pid).is_some(),
            "parked process {pid} exited instead of suspending"
        );
    }

    // THE PROPERTY: with all of those parked, the single normal scheduler
    // thread is still free to run other work to completion.
    let progress_pid = scheduler.spawn_process(&progress_module);
    let (reason, result) = run_until_exit_bounded(&scheduler, progress_pid);
    assert_eq!(reason, ExitReason::Normal);
    assert_eq!(result.root(), Term::small_int(7));
    assert_eq!(
        NORMAL_PROGRESS.load(Ordering::Acquire),
        1,
        "the progress native never ran: the normal scheduler thread was retired"
    );

    // ...and no DIRTY thread was consumed either. An Io-marked blocking BIF
    // (the rejected shortcut) allocates one dirty suspension call id and one
    // transient completion thread per call; both instruments stay flat here.
    assert_eq!(
        scheduler.dirty_suspension_allocation_count(),
        dirty_allocations_before,
        "a sleep_forever park allocated a dirty suspension: it is not running in-slice"
    );
    assert_eq!(
        completion_spawned_total(&scheduler),
        dirty_completions_before,
        "a sleep_forever park spawned a dirty completion thread"
    );

    // Still parked afterwards — the progress run did not wake or drain them.
    for pid in &parked {
        assert!(
            scheduler.process_table().get(*pid).is_some(),
            "parked process {pid} was woken by unrelated scheduler activity"
        );
    }

    // The park is TOTAL, not merely idle: a message arrival is accepted (the
    // process is live and addressable) but must not un-park it. The mirror is
    // registered `wake_on_message: false`, so `suspension_blocks_wake` gates
    // the wake out. A message-wakeable park would turn every incoming message
    // into a slice that re-runs the native and re-parks.
    let probe_target = parked[0];
    assert!(
        scheduler.enqueue_atom_message(probe_target, Atom::OK),
        "the parked process should still be a live message target"
    );
    let mirrors_after_park = scheduler.suspension_mirror_registration_count();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        scheduler.process_table().get(probe_target).is_some(),
        "a plain message terminated the parked process"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_after_park,
        "a plain message woke the parked process and it re-registered a suspension"
    );

    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// (b) A park is not a leak: the process stays killable from outside.
// ---------------------------------------------------------------------------

#[test]
fn a_parked_process_remains_subject_to_exit_signals() {
    let registry = Arc::new(ModuleRegistry::new());
    let parked_module = registry.insert(call_native_module(
        Atom::OK,
        native_import(bif_sleep_forever, Capability::Clock),
    ));

    let scheduler = scheduler_with_one_normal_thread(&registry);

    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let pid = scheduler.spawn_process(&parked_module);
    wait_for_parks(&scheduler, mirrors_before, 1);

    assert!(
        scheduler.process_table().get(pid).is_some(),
        "process {pid} exited instead of parking, so the kill below would prove nothing"
    );

    // Kill it from outside, exactly as `erlang:exit/2` would. A `Waiting`
    // process is terminated through its slot; it does not need to be woken.
    scheduler
        .exit_signal(0, pid, ExitReason::Kill)
        .expect("exit signal to the parked process");

    poll_until(
        PARK_TIMEOUT,
        "the parked process survived a kill exit signal",
        || scheduler.process_table().get(pid).is_none().then_some(()),
    );

    scheduler.shutdown();
}
