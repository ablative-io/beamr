//! ADMISSION ARC LEG 1b (CFG-sensitive guard) — blocking-receive un-walling and
//! the honest demand-path limit.
//!
//! Condition 4 of the CFG ruling: a single blocking receive is no longer
//! wholesale walled. The PEEK/PARK family (loop_rec/loop_rec_end/wait/wait_timeout)
//! is pure, so a FRAMELESS blocking receive ADMITS — proven directly at the
//! compiler level by `wait_timeout_and_blocking_receive_lower_under_path_sensitivity`
//! and the first compile of `selective_receive_peek_lowers_and_send_then_receive_is_walled`.
//!
//! This test carries the honest demand-path finding for a REAL OTP-29 erlc
//! receive: erlc FRAMES the receive (`allocate`/`deallocate` around it), and
//! `deallocate` — a deopt-capable frame teardown — sits after `remove_message`
//! (the consume) on the matched path. That is the SAME message-loss class as the
//! two-sequential-receives wall (a deopt at the teardown would restart a function
//! whose receive already consumed a message), so echo/0 stays an honest
//! rejection. The un-walling is real for the peek/park; the frame teardown is the
//! honest remaining wall — NOT forced (per the ruling's "if still walled, report
//! why"). The whole-function slicer path (AOT) is the same admission machinery
//! the demand path drives.

use std::path::PathBuf;

use beamr::jit::AotCompiler;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn real_erlc_blocking_receive_is_walled_by_the_frame_teardown_after_consume() {
    let compiler = AotCompiler::new().expect("AOT compiler builds");
    let result = compiler
        .compile_module(&fixture("recv_jit.beam"))
        .expect("recv_jit compiles");

    // echo/0 is `receive M -> M end` — a single blocking receive. erlc frames it,
    // so `deallocate` (deopt-capable) follows `remove_message` on the matched
    // path. The CFG-sensitive guard walls it as the frame-teardown loss class —
    // the honest reason, revealed by the dataflow, that a real-erlc receive stays
    // rejected even though the peek/park itself is now pure.
    let framed_receive_wall = result.skipped_functions().iter().any(|(_, _, reason)| {
        reason.contains("Deallocate") && reason.contains("after an observable side effect")
    });
    assert!(
        framed_receive_wall,
        "a real-erlc blocking receive must be walled by deallocate-after-consume \
         (the honest frame-teardown loss class), got: {:?}",
        result.skipped_functions()
    );
}
