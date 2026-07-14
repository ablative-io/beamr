use std::fs;

#[test]
fn generated_bootstrap_uses_await_exit_without_a_step_loop() {
    let source =
        fs::read_to_string(env!("BEAMR_WASM_BOOTSTRAP")).expect("generated bootstrap is readable");

    assert!(source.contains("export async function awaitExit(vm, pid)"));
    assert!(source.contains("await vm.await_exit(pid)"));
    assert!(!source.contains("runUntilExit"));
    assert!(!source.contains("maxSteps"));
    assert!(!source.contains("vm.run_step()"));
}
