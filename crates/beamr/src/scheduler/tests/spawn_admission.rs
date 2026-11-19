//! Spawns after teardown: every facility method refuses, and shutdown waits
//! for a spawn already admitted.

use super::*;

/// Round-3 major: EVERY `SpawnFacility` method refuses after teardown — not
/// just `spawn`. A dirty native surviving on an embedder-owned shared pool
/// can reach any of them through its retained context.
#[test]
fn every_spawn_facility_method_refuses_after_teardown() {
    use crate::native::SpawnFacility as _;
    use crate::native::spawn::SpawnError;

    let atoms = AtomTable::new();
    let module_name = atoms.intern("teardown_refusal");
    let function = atoms.intern("main");
    let mut module = test_module(module_name, vec![Instruction::Label { label: 7 }]);
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let _module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let processes_before = scheduler.shared.process_bodies.len();

    assert_eq!(
        facility.spawn(1, module_name, function, Vec::new(), None),
        Err(SpawnError::SchedulerTearingDown)
    );
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    assert_eq!(
        facility.spawn_native(
            1,
            Box::new(move || {
                Box::new(BusyPollHandler {
                    received: Arc::clone(&factory_received),
                    slices: Arc::clone(&factory_slices),
                })
            }),
            None
        ),
        Err(SpawnError::SchedulerTearingDown)
    );
    assert!(matches!(
        facility.spawn_monitor(1, module_name, function, Vec::new()),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda(1, module_name, 0, None),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda_monitor(1, module_name, 0),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_with_options(
            1,
            module_name,
            function,
            Vec::new(),
            SpawnOptions::default()
        ),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda_with_options(1, module_name, 0, SpawnOptions::default()),
        Err(SpawnError::SchedulerTearingDown)
    ));

    assert_eq!(
        scheduler.shared.process_bodies.len(),
        processes_before,
        "no refused spawn path touched process state"
    );
}

struct AdmittedSpawnObservation {
    body_present: bool,
    table_present: bool,
    record_present: bool,
    completion: ExitCompletionWatchState,
}

struct AdmittedSpawnProbe {
    admitted: std::sync::mpsc::Receiver<()>,
    publisher_release: Arc<std::sync::Barrier>,
    waiting: std::sync::mpsc::Receiver<()>,
    before_drain: std::sync::mpsc::Receiver<AdmittedSpawnObservation>,
    drain_release: Arc<std::sync::Barrier>,
}

fn admitted_spawn_probe(scheduler: &Arc<Scheduler>, pid: u64) -> AdmittedSpawnProbe {
    let (admitted_tx, admitted) = std::sync::mpsc::channel();
    let publisher_release = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::clone(&publisher_release);
    scheduler
        .shared
        .spawn_admission_hook
        .set(Box::new(move || {
            admitted_tx
                .send(())
                .unwrap_or_else(|error| panic!("admission signal: {error}"));
            release.wait();
        }))
        .unwrap_or_else(|_| panic!("admission hook already installed"));
    let (waiting_tx, waiting) = std::sync::mpsc::channel();
    scheduler
        .shared
        .teardown_wait_hook
        .set(Box::new(move || {
            waiting_tx
                .send(())
                .unwrap_or_else(|error| panic!("drain wait signal: {error}"));
        }))
        .unwrap_or_else(|_| panic!("drain wait hook already installed"));
    let (stored_tx, before_drain) = std::sync::mpsc::channel();
    let drain_release = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::clone(&drain_release);
    let weak = Arc::downgrade(scheduler);
    let observed = AtomicBool::new(false);
    scheduler
        .shared
        .terminal_admission_hook
        .set(Box::new(move || {
            if observed.swap(true, Ordering::SeqCst) {
                return;
            }
            let scheduler = weak
                .upgrade()
                .unwrap_or_else(|| panic!("scheduler owner remains live"));
            stored_tx
                .send(AdmittedSpawnObservation {
                    body_present: scheduler.shared.process_bodies.contains_key(&pid),
                    table_present: scheduler.process_table().get(pid).is_some(),
                    record_present: scheduler.shared.exit_tombstones.contains_key(&pid),
                    completion: scheduler.watch_exit_completion(pid),
                })
                .unwrap_or_else(|error| panic!("publication observation: {error}"));
            release.wait();
        }))
        .unwrap_or_else(|_| panic!("terminal drain hook already installed"));
    AdmittedSpawnProbe {
        admitted,
        publisher_release,
        waiting,
        before_drain,
        drain_release,
    }
}

fn admitted_spawn_scheduler() -> (Arc<Scheduler>, Atom, Atom) {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("admitted_spawn");
    let function = atoms.intern("main");
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Wait {
                fail: Operand::Label(7),
            },
        ],
    );
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let _module = registry.insert(module);
    let scheduler = Arc::new(
        Scheduler::with_services(
            SchedulerConfig {
                thread_count: Some(1),
                ..SchedulerConfig::default()
            },
            SchedulerServices::minimal(),
            registry,
            NativeBifs::none(),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );
    (scheduler, module_name, function)
}

#[test]
fn shutdown_waits_for_an_admitted_spawn_and_refuses_the_next() {
    use crate::native::spawn::SpawnError;

    let (scheduler, module_name, function) = admitted_spawn_scheduler();
    let expected_pid = scheduler.shared.next_pid.load(Ordering::Relaxed);
    let probe = admitted_spawn_probe(&scheduler, expected_pid);
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let spawn_thread =
        std::thread::spawn(move || facility.spawn(1, module_name, function, Vec::new(), None));
    probe
        .admitted
        .recv()
        .unwrap_or_else(|error| panic!("spawn is admitted: {error}"));
    let shutdown_done = Arc::new(AtomicBool::new(false));
    let closing = Arc::clone(&scheduler);
    let completed = Arc::clone(&shutdown_done);
    let shutdown_thread = std::thread::spawn(move || {
        closing.shutdown();
        completed.store(true, Ordering::Release);
    });
    probe
        .waiting
        .recv()
        .unwrap_or_else(|error| panic!("shutdown waits for publisher: {error}"));
    let premature_close = shutdown_done.load(Ordering::Acquire);
    probe.publisher_release.wait();
    let spawn_result = spawn_thread.join();
    let before_drain = probe
        .before_drain
        .recv()
        .unwrap_or_else(|error| panic!("joined publication observation: {error}"));
    probe.drain_release.wait();
    shutdown_thread
        .join()
        .unwrap_or_else(|_| panic!("shutdown thread joins"));
    let spawned = spawn_result
        .unwrap_or_else(|_| panic!("spawn thread joins"))
        .unwrap_or_else(|error| panic!("the admitted spawn completes: {error:?}"));
    assert!(!premature_close, "shutdown waits for the admitted spawn");
    assert!(shutdown_done.load(Ordering::Acquire));
    assert_eq!(spawned, expected_pid);
    assert!(before_drain.body_present);
    assert!(before_drain.table_present);
    assert!(!before_drain.record_present);
    assert!(!scheduler.shared.process_bodies.contains_key(&spawned));
    assert!(scheduler.process_table().get(spawned).is_none());
    let outcome = scheduler
        .take_exit_outcome(spawned)
        .unwrap_or_else(|| panic!("joined drain retains the genuine outcome"));
    assert_eq!(outcome.0, ExitReason::Killed);
    let completion = match before_drain.completion {
        ExitCompletionWatchState::Live(watch) => watch,
        other => panic!("published process has a live completion watch: {other:?}"),
    };
    assert_eq!(completion.recv(), Ok((spawned, outcome.0)));
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    assert_eq!(
        facility.spawn(1, module_name, function, Vec::new(), None),
        Err(SpawnError::SchedulerTearingDown)
    );
}
