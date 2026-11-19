//! Signals and messages to a process checked out on a worker, and the
//! sentinel links merged back at store-back.

use super::*;

#[test]
fn exit_signal_tombstones_executing_non_trapping_process() {
    let shared = make_shared_state();
    let parent = insert_process(&shared, 1);
    let child = insert_process(&shared, 2);
    add_link(&shared, parent, child);

    let process = make_executing(&shared, child);

    cleanup_exited_process(&shared, parent, ExitReason::Error);

    assert!(
        shared.exit_tombstones.contains_key(&child),
        "executing child should receive a tombstone"
    );
    store_runnable_process(&shared, process);
    assert!(cleanup_if_tombstoned_after_store(&shared, child));
    assert!(
        !is_alive(&shared, child),
        "tombstoned child should be cleaned"
    );
}

#[test]
fn exit_signal_queues_message_for_executing_trapping_process() {
    let shared = make_shared_state();
    let parent = insert_process(&shared, 1);
    let child = insert_process(&shared, 2);
    add_link(&shared, parent, child);
    set_trap_exit(&shared, child, true);

    let process = make_executing(&shared, child);

    cleanup_exited_process(&shared, parent, ExitReason::Error);

    assert!(
        !shared.exit_tombstones.contains_key(&child),
        "trapping child should not be tombstoned"
    );
    store_runnable_process(&shared, process);
    let msg = read_mailbox_tuple(&shared, child)
        .unwrap_or_else(|| panic!("pending EXIT message delivered on store-back"));
    assert_eq!(msg[0], Term::atom(Atom::EXIT));
    assert_eq!(msg[1], Term::pid(parent));
    assert_eq!(msg[2], Term::atom(Atom::ERROR));
}

#[test]
fn normal_exit_signal_queues_message_for_executing_trapping_process() {
    // Regression for the Executing-arm normal-exit drop bug: a NORMAL linked
    // exit to a trapping process that is MID-SLICE (`ProcessSlot::Executing`)
    // MUST still deliver `{'EXIT', Pid, normal}`, exactly as the Present arm
    // does. The Executing arm previously gated message delivery on
    // `reason != Normal`, so it silently dropped `{EXIT, src, normal}` —
    // diverging from OTP (a trapping process receives `{'EXIT', Pid, normal}`
    // for a normal linked exit) and from the Present arm. The child must NOT be
    // tombstoned or killed (a normal exit never kills), and the message must be
    // drained into the mailbox at the slice boundary by `store_runnable_process`.
    let shared = make_shared_state();
    let parent = insert_process(&shared, 1);
    let child = insert_process(&shared, 2);
    add_link(&shared, parent, child);
    set_trap_exit(&shared, child, true);

    let process = make_executing(&shared, child);

    cleanup_exited_process(&shared, parent, ExitReason::Normal);

    assert!(
        !shared.exit_tombstones.contains_key(&child),
        "a normal exit must not tombstone a linked trapping executing child"
    );
    assert!(
        is_alive(&shared, child),
        "a trapped normal exit must not kill the executing child"
    );
    store_runnable_process(&shared, process);
    let msg = read_mailbox_tuple(&shared, child)
        .unwrap_or_else(|| panic!("normal EXIT message must be delivered on store-back"));
    assert_eq!(msg[0], Term::atom(Atom::EXIT));
    assert_eq!(msg[1], Term::pid(parent));
    assert_eq!(msg[2], Term::atom(Atom::NORMAL));
}

#[test]
fn exit_signal_normal_queues_message_for_executing_trapping_process() {
    // Regression for the `exit_signal` (erlang:exit/2) Executing-arm normal-exit
    // drop bug: `exit_signal(From, Target, normal)` to a trapping process that is
    // MID-SLICE (`ProcessSlot::Executing`) MUST still deliver `{'EXIT', From,
    // normal}`, exactly as the Present arm does. The Executing arm previously
    // gated message delivery on `reason != Normal`, silently dropping
    // `{EXIT, From, normal}` — diverging from OTP (a trapping process receives
    // `{'EXIT', Pid, normal}` for `erlang:exit/2` with reason `normal`) and from
    // the Present arm. The target must NOT be tombstoned or killed (a normal exit
    // never kills), and the message must be drained into the mailbox at the slice
    // boundary by `store_runnable_process`. Exercises the
    // `SchedulerSupervisionFacility::exit_signal` path (NOT the link cascade).
    use crate::native::supervision::SupervisionFacility;

    let shared = make_shared_state();
    let from = insert_process(&shared, 1);
    let target = insert_process(&shared, 2);
    set_trap_exit(&shared, target, true);

    let process = make_executing(&shared, target);

    let facility = supervision_integration::SchedulerSupervisionFacility {
        shared: Arc::clone(&shared),
    };
    facility
        .exit_signal(from, target, ExitReason::Normal)
        .unwrap_or_else(|_| panic!("exit_signal must succeed"));

    assert!(
        !shared.exit_tombstones.contains_key(&target),
        "a normal exit_signal must not tombstone a trapping executing target"
    );
    assert!(
        is_alive(&shared, target),
        "a trapped normal exit_signal must not kill the executing target"
    );
    store_runnable_process(&shared, process);
    let msg = read_mailbox_tuple(&shared, target)
        .unwrap_or_else(|| panic!("normal EXIT message must be delivered on store-back"));
    assert_eq!(msg[0], Term::atom(Atom::EXIT));
    assert_eq!(msg[1], Term::pid(from));
    assert_eq!(msg[2], Term::atom(Atom::NORMAL));
}

#[test]
fn take_links_from_reads_executing_sentinel_links() {
    let shared = make_shared_state();
    let source = insert_process(&shared, 1);
    let linked = insert_process(&shared, 2);
    add_link(&shared, source, linked);

    let _process = make_executing(&shared, source);

    let links = supervision_integration::take_links_from(&shared, source);
    assert_eq!(links, vec![linked]);
}

#[test]
fn sentinel_links_merge_into_body_on_store_back() {
    let shared = make_shared_state();
    let pid = insert_process(&shared, 1);
    let linked = insert_process(&shared, 2);
    let process = make_executing(&shared, pid);

    {
        let entry = shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Executing(metadata) = &mut *slot else {
            panic!("slot is executing");
        };
        metadata.add_link(linked, pid);
    }

    store_runnable_process(&shared, process);

    assert!(process_links_contain(&shared, pid, linked));
}

/// BLOCKER regression: a reference-bearing message sent to an *Executing*
/// receiver must survive the ETF encode -> store-back decode round trip. Before
/// the fix, decode.rs had no reference arm, so the message decoded as an error
/// and was silently dropped at the store-back `else { continue; }`. This test
/// drives the real `SchedulerLocalSendFacility` (via `build_native_services`)
/// against an Executing receiver — i.e. it exercises the
/// `ProcessSlot::Executing` ETF deferral arm — then stores the receiver back
/// and asserts the `{Ref, ping}` tuple (with an intact ref id) is delivered.
#[test]
fn reference_message_to_executing_receiver_round_trips_via_etf() {
    let shared = make_shared_state();
    let sender_pid = insert_process(&shared, 1);
    let receiver_pid = insert_process(&shared, 2);

    // Build `{Ref, ping}` on a live sender heap; `send_local` reads the term
    // during the call, so this Process must outlive the call.
    let ping = shared.atom_table.intern("ping");
    let mut sender = Process::new(sender_pid, 64);
    let ref_id = 0x0123_4567_89ab_cdef_u64;
    let ref_ptr = sender.heap_mut().alloc(2).expect("reference words");
    // SAFETY: alloc returned exactly the two words write_reference needs.
    let ref_words = unsafe { std::slice::from_raw_parts_mut(ref_ptr, 2) };
    let reference = boxed::write_reference(ref_words, ref_id).expect("reference fits");
    let tuple_ptr = sender.heap_mut().alloc(3).expect("tuple words");
    // SAFETY: alloc returned exactly the three words write_tuple needs.
    let tuple_words = unsafe { std::slice::from_raw_parts_mut(tuple_ptr, 3) };
    let message =
        boxed::write_tuple(tuple_words, &[reference, Term::atom(ping)]).expect("tuple fits");

    // Force the receiver into the Executing branch so the ETF deferral path runs.
    let receiver = make_executing(&shared, receiver_pid);

    let services = supervision_integration::build_native_services(&shared, NamespaceId::DEFAULT);
    let facility = services.local_send.expect("local send facility wired");
    facility
        .send_local(crate::native::local_send::LocalSendRequest {
            target_pid: receiver_pid,
            sender_pid,
            message,
            sender_clock: 1,
            replay_driver: None,
        })
        .expect("local send to executing receiver succeeds");

    // Store the receiver back: this drains pending_local_messages, decoding the
    // ETF payload onto the receiver heap.
    store_runnable_process(&shared, receiver);

    let msg = read_mailbox_tuple(&shared, receiver_pid)
        .unwrap_or_else(|| panic!("executing receiver gets the ref-bearing message"));
    assert_eq!(msg.len(), 2, "message should be a 2-tuple {{Ref, ping}}");
    let decoded_ref =
        boxed::Reference::new(msg[0]).unwrap_or_else(|| panic!("first element is a reference"));
    assert_eq!(
        decoded_ref.id(),
        ref_id,
        "reference id must survive the ETF round trip through the Executing path"
    );
    assert_eq!(msg[1], Term::atom(ping));

    // Keep the sender heap alive until after the facility call has copied/encoded.
    drop(sender);
}
