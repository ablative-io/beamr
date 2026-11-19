//! process_info read from a running process's metadata.

use super::*;

#[test]
fn process_info_reads_executing_process_metadata() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("executing_info");
    let function = atoms.intern("main");
    // func_info bounds `main/0` from ip 0; the process is positioned at ip 1
    // (inside those bounds) so the Present->Executing snapshot DERIVES the MFA.
    let mut module = test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
        ],
    );
    module.function_table = vec![(0, function, 0)];
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
    scheduler.shutdown();
    let pid = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    {
        let entry = scheduler
            .shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process body exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("test process should be present");
        };
        process.set_code_position(Some(CodePosition {
            module: module_name,
            instruction_pointer: 1,
        }));
    }

    let process = take_runnable_process(&scheduler.shared, pid)
        .unwrap_or_else(|| panic!("process should transition to executing"));

    assert_eq!(
        scheduler
            .shared
            .process_info(pid, ProcessInfoItem::CurrentFunction),
        Some(ProcessInfoValue::CurrentFunction(Some((
            module_name,
            function,
            0
        ))))
    );
    assert_eq!(
        scheduler.shared.process_info(pid, ProcessInfoItem::Status),
        Some(ProcessInfoValue::Status(ProcessInfoStatus::Running))
    );

    store_runnable_process(&scheduler.shared, process);
}

#[test]
fn process_info_current_function_is_derived_from_module_and_ip() {
    // Fail-first (current_mfa lane #1): a process parked mid-function carries
    // `current_module` + `code_position` but NO stored MFA (func_info, its only
    // writer, runs only on dispatch failure). process_info(current_function)
    // must DERIVE the MFA from the module's func_info bounds at the current ip.
    // RED on `main`: the stale/None stored field answers `{undefined,undefined,0}`.
    let atoms = AtomTable::new();
    let module_name = atoms.intern("parked_mod");
    let waiter = atoms.intern("waiter");
    // A module whose func_info table bounds `waiter/0` from ip 0 onward. The
    // process is positioned at ip 1 (inside those bounds), as a receive-parked
    // process would be between scheduler slices.
    let mut module = test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(waiter)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
        ],
    );
    module.function_table = vec![(0, waiter, 0)];
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
    scheduler.shutdown();
    let pid = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    {
        let entry = scheduler
            .shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process body exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("test process should be present");
        };
        process.set_code_position(Some(CodePosition {
            module: module_name,
            instruction_pointer: 1,
        }));
    }

    assert_eq!(
        scheduler
            .shared
            .process_info(pid, ProcessInfoItem::CurrentFunction),
        Some(ProcessInfoValue::CurrentFunction(Some((
            module_name,
            waiter,
            0
        )))),
        "current_function must derive `waiter/0` from (module, ip), not answer undefined"
    );
}
