//! Spawning through the facility: link, namespace and group-leader
//! inheritance, spawn options, and capability restriction.

use super::*;

#[test]
fn linked_test_spawn_inherits_parent_group_leader_not_child_pid() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();
    let parent = scheduler.spawn_test_process(false);
    let parent_group_leader = Term::pid(77);
    assert!(scheduler.set_test_group_leader(parent, parent_group_leader));

    let child = scheduler
        .spawn_linked_test_process(parent)
        .unwrap_or_else(|error| panic!("linked child starts: {error}"));

    assert_eq!(
        scheduler.test_group_leader(child),
        Some(parent_group_leader)
    );
    assert_ne!(scheduler.test_group_leader(child), Some(Term::pid(child)));
}

#[test]
fn spawn_link_uses_executing_parent_namespace_and_merges_parent_link() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_link_child");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let namespace = scheduler.create_namespace();
    let namespace_registry = scheduler
        .shared
        .namespace_store
        .get(&namespace)
        .map(|entry| Arc::clone(&entry))
        .unwrap_or_else(|| panic!("namespace registry exists"));
    // The child PARKS on Wait (label 7): its body persists for the namespace
    // and link assertions without racing the live worker. (This test formerly
    // used shutdown-as-barrier and spawned afterwards; post-shutdown spawns
    // are now REFUSED by the §4 teardown gate, so the barrier is the parked
    // child instead.)
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
    let module = namespace_registry.insert(module);
    let parent = scheduler.spawn_test_process_in(namespace, Arc::clone(&module));

    let process = take_runnable_process(&scheduler.shared, parent)
        .unwrap_or_else(|| panic!("parent body taken"));

    let child = scheduler
        .spawn_link(parent, module_name, function, Vec::new())
        .unwrap_or_else(|error| panic!("spawn_link succeeds with executing parent: {error:?}"));

    assert_eq!(scheduler.process_namespace(parent), Some(namespace));
    assert_eq!(scheduler.process_namespace(child), Some(namespace));
    assert!(process_links_contain(&scheduler.shared, parent, child));
    store_runnable_process(&scheduler.shared, process);
    assert!(scheduler.is_linked(parent, child));
    scheduler.shutdown();
}

/// Wait (bounded) until `pid` is Present and parked Waiting — used by tests
/// whose children park on a `Wait` instruction so their bodies stay stably
/// inspectable with the worker live (post-shutdown spawns are refused by the
/// §4 teardown gate, so shutdown-as-barrier-then-spawn is no longer legal).
fn wait_until_parked(shared: &SharedState, pid: u64) {
    wait_until(10_000, || {
        shared.process_bodies.get(&pid).is_some_and(|entry| {
            let slot = lock_or_recover(&entry);
            matches!(
                &*slot,
                ProcessSlot::Present(ScheduledProcess(process))
                    if process.status() == ProcessStatus::Waiting
            )
        })
    });
}

#[test]
fn spawn_facility_options_apply_link_monitor_priority_and_heap_before_wake() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_opt_scheduler");
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
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let parent = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };

    let result = facility
        .spawn_with_options(
            parent,
            module_name,
            function,
            Vec::new(),
            SpawnOptions {
                link: true,
                monitor: true,
                priority: Some(Priority::High),
                min_heap_size: Some(512),
                capabilities: None,
            },
        )
        .unwrap_or_else(|error| panic!("spawn_with_options succeeds: {error:?}"));
    // The child parks on Wait; from then on its body is stably Present.
    wait_until_parked(&scheduler.shared, result.pid);

    assert!(scheduler.is_linked(parent, result.pid));
    let parent_entry = scheduler
        .shared
        .process_bodies
        .get(&parent)
        .unwrap_or_else(|| panic!("parent body exists"));
    let parent_slot = lock_or_recover(&parent_entry);
    let ProcessSlot::Present(ScheduledProcess(parent_process)) = &*parent_slot else {
        panic!("parent process should be present");
    };
    let reference = result.reference.expect("monitor reference");
    assert!(parent_process.links().contains(&result.pid));
    assert!(
        parent_process
            .monitors()
            .iter()
            .any(|monitor| monitor.reference() == reference
                && monitor.watcher() == parent
                && monitor.target() == result.pid)
    );
    drop(parent_slot);
    drop(parent_entry);

    let child_entry = scheduler
        .shared
        .process_bodies
        .get(&result.pid)
        .unwrap_or_else(|| panic!("child body exists"));
    let child_slot = lock_or_recover(&child_entry);
    let ProcessSlot::Present(ScheduledProcess(child_process)) = &*child_slot else {
        panic!("child process should be present");
    };
    assert_eq!(child_process.priority(), Priority::High);
    assert_eq!(child_process.heap().capacity(), 512);
    assert!(child_process.links().contains(&parent));
    assert!(
        child_process
            .monitors()
            .iter()
            .any(|monitor| monitor.reference() == reference
                && monitor.watcher() == parent
                && monitor.target() == result.pid)
    );
    drop(child_slot);
    drop(child_entry);
    scheduler.shutdown();
}

#[test]
fn spawn_facility_restricts_child_to_explicit_capabilities() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_opt_capability_scheduler");
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
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let parent = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let restricted = CapabilitySet::from_slice(&[Capability::Pure, Capability::ProcessLocal]);

    let result = facility
        .spawn_with_options(
            parent,
            module_name,
            function,
            Vec::new(),
            SpawnOptions {
                capabilities: Some(restricted.clone()),
                ..SpawnOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("spawn_with_options succeeds: {error:?}"));
    // The child parks on Wait; from then on its body is stably Present.
    wait_until_parked(&scheduler.shared, result.pid);

    let child_entry = scheduler
        .shared
        .process_bodies
        .get(&result.pid)
        .unwrap_or_else(|| panic!("child body exists"));
    let child_slot = lock_or_recover(&child_entry);
    let ProcessSlot::Present(ScheduledProcess(child_process)) = &*child_slot else {
        panic!("child process should be present");
    };
    assert_eq!(child_process.capabilities(), &restricted);
    assert!(
        !child_process
            .capabilities()
            .contains(Capability::ExternalIo)
    );
}
