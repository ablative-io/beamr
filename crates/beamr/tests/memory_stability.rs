use beamr::{gc::collect_major, native::ProcessContext, process::Process, term::Term};

const ITERATIONS: usize = 10_000;

#[test]
fn bif_and_literal_heavy_workload_does_not_grow_monotonically() {
    let mut process = Process::new(52, 65_536);
    process.heap_mut().set_max_capacity(262_144);

    for index in 0..ITERATIONS {
        let random_like_bif_value = synthetic_rand_uniform(&mut process, index);
        let literal = materialise_literal(&mut process, b"literal-heavy-payload");
        let tuple = materialise_tuple(&mut process, random_like_bif_value, literal);
        let list = materialise_list(&mut process, &[tuple, literal, random_like_bif_value]);

        // Simulate send-to-self traffic without retaining O(N) mailbox roots: enqueue the
        // message-shaped value in X0 for the duration of the iteration, then consume it.
        process.set_x_reg(0, list);
        process.set_x_reg(0, Term::NIL);
    }

    for register in process.x_regs_mut().iter_mut() {
        *register = Term::NIL;
    }

    collect_major(&mut process).expect("full GC after transient workload should succeed");

    assert!(
        process.heap().total_used() <= 512,
        "heap after full GC should remain bounded independently of {ITERATIONS} iterations; used {} words",
        process.heap().total_used()
    );
}

fn synthetic_rand_uniform(process: &mut Process, index: usize) -> Term {
    let mut context = ProcessContext::with_process_heap(process.pid(), process.heap_mut());
    let value = ((index % 997) as f64 + 1.0) / 997.0;
    context
        .alloc_float(value)
        .expect("rand:uniform/0-style float allocation should fit")
}

fn materialise_literal(process: &mut Process, bytes: &[u8]) -> Term {
    let mut context = ProcessContext::with_process_heap(process.pid(), process.heap_mut());
    context
        .alloc_binary(bytes)
        .expect("literal binary allocation should fit")
}

fn materialise_tuple(process: &mut Process, random: Term, literal: Term) -> Term {
    let mut context = ProcessContext::with_process_heap(process.pid(), process.heap_mut());
    context
        .alloc_tuple(&[Term::small_int(3), random, literal])
        .expect("tuple allocation should fit")
}

fn materialise_list(process: &mut Process, elements: &[Term]) -> Term {
    let mut context = ProcessContext::with_process_heap(process.pid(), process.heap_mut());
    context
        .alloc_list(elements)
        .expect("list allocation should fit")
}
