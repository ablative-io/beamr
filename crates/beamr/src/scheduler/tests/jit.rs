//! The JIT a scheduler builds: its threshold, the operator switch, how replay
//! and a disabled dirty pool compose it away, and constructor failure.

use super::*;

#[test]
fn scheduler_uses_explicit_jit_threshold() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            jit_threshold: Some(500),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");

    assert_eq!(scheduler.jit_profiler().current_threshold(), 500);
    scheduler.shutdown();
}

/// R1's three gating facts, pinned at the site that holds them: the JIT
/// profiling handle is present IFF the jit feature is compiled (this test IS
/// jit-compiled), replay mode is off, and the dirty-CPU service is live.
#[test]
fn replay_and_disabled_dirty_cpu_compose_the_jit_handle_away() {
    let live = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("live scheduler starts");
    let services =
        supervision_integration::build_native_services(&live.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_some(),
        "default profile with a live dirty-CPU pool must offer the profiling handle"
    );
    live.shutdown();

    let replay = Scheduler::new_replay(
        SchedulerConfig::default(),
        ReplayLog::default(),
        NativeBifs::none(),
    )
    .expect("replay scheduler starts");
    let services =
        supervision_integration::build_native_services(&replay.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_none(),
        "replay composes the profiling handle away entirely"
    );
    replay.shutdown();

    let minimal = Scheduler::with_services(
        SchedulerConfig::default(),
        SchedulerServices::minimal(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("minimal scheduler starts");
    let services =
        supervision_integration::build_native_services(&minimal.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_none(),
        "a Disabled dirty-CPU service composes the profiling handle away"
    );
    minimal.shutdown();
}

/// The operator switch composes BOTH JIT handles away — the half that matters
/// twice over.
///
/// Withholding the profiling handle is what stops compilation. Withholding the
/// CACHE is what stops already-compiled code being ENTERED, and that is the
/// half a switch could plausibly get wrong while still looking like it worked:
/// a scheduler that stopped compiling but kept dispatching into code compiled
/// before the switch was thrown would leave an operator convinced the JIT was
/// off while the defect they were escaping ran on. Cache PRESENCE is exactly
/// the variable that distinguishes those two worlds, so it is asserted
/// directly at the composition point rather than inferred from behaviour.
///
/// The enabled arm is the positive control: it proves this test can see the
/// handles at all, so the disabled arm's absence is a measurement and not a
/// dead instrument.
#[test]
fn the_operator_switch_composes_away_both_the_cache_and_the_profiling_handle() {
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler starts");

    // Positive control: on by default, both handles present.
    assert!(scheduler.jit_enabled(), "the switch defaults to on");
    let services =
        supervision_integration::build_native_services(&scheduler.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_cache.is_some() && services.jit_profiling.is_some(),
        "control: an enabled scheduler must offer BOTH handles, or this test \
         cannot detect their absence"
    );

    let previous = scheduler.set_jit_enabled(false);
    assert!(previous, "the setter returns the previous setting");
    assert!(!scheduler.jit_enabled());
    let services =
        supervision_integration::build_native_services(&scheduler.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_none(),
        "the switch composes the profiling handle away: nothing may compile"
    );
    assert!(
        services.jit_cache.is_none(),
        "the switch composes the CACHE away: already-compiled code may not be \
         entered, not merely no new code produced"
    );

    // Reversible: the switch withholds the handles, it does not destroy them.
    let previous = scheduler.set_jit_enabled(true);
    assert!(!previous);
    let services =
        supervision_integration::build_native_services(&scheduler.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_cache.is_some() && services.jit_profiling.is_some(),
        "re-enabling restores both handles"
    );
    scheduler.shutdown();
}

#[test]
fn failing_jit_compiler_construction_surfaces_from_the_scheduler_constructor() {
    INJECT_JIT_COMPILER_FAILURE.with(|flag| flag.set(true));
    let result = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    );
    INJECT_JIT_COMPILER_FAILURE.with(|flag| flag.set(false));

    match result {
        Err(error) => assert!(
            error.contains("JIT compiler construction failed"),
            "constructor error must name the JIT compiler: {error}"
        ),
        Ok(scheduler) => {
            scheduler.shutdown();
            panic!("a failing JitCompiler::new must fail scheduler construction, not degrade");
        }
    }
}

#[test]
fn delete_module_drops_jit_profile_entries() {
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");
    let module = scheduler.shared.atom_table.intern("jit_deleted_module");
    let function = scheduler.shared.atom_table.intern("hot");
    // Entries are born at a live call edge; completions only update them.
    scheduler.jit_profiler().record_call(module, function, 0, 1);
    let epoch = scheduler
        .jit_profiler()
        .profile_epoch(module, function, 0)
        .expect("profile exists");
    scheduler
        .jit_profiler()
        .mark_compiled(module, function, 0, 1, epoch);
    assert!(scheduler.jit_profiler().is_compiled(module, function, 0));

    scheduler.delete_module(module);

    assert!(
        !scheduler.jit_profiler().is_compiled(module, function, 0),
        "delete_module must drop the module's JIT profile entries"
    );
    scheduler.shutdown();
}
