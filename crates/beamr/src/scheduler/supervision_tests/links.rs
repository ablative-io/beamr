//! Exit propagation through links: error, normal, kill and trapped exits,
//! cascades, and links across namespaces.

use super::*;

#[test]
fn linked_process_dies_on_error_exit() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    add_link(&shared, a, b);

    cleanup_exited_process(&shared, a, ExitReason::Error);

    assert!(!is_alive(&shared, a), "process A should be removed");
    assert!(
        !is_alive(&shared, b),
        "linked process B should die from error exit"
    );
}

#[test]
fn linked_process_survives_normal_exit() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    add_link(&shared, a, b);

    cleanup_exited_process(&shared, a, ExitReason::Normal);

    assert!(!is_alive(&shared, a), "process A should be removed");
    assert!(
        is_alive(&shared, b),
        "linked process B should survive normal exit"
    );
}

#[test]
fn trap_exit_receives_exit_message() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    add_link(&shared, a, b);
    set_trap_exit(&shared, b, true);

    cleanup_exited_process(&shared, a, ExitReason::Error);

    assert!(!is_alive(&shared, a), "process A should be removed");
    assert!(is_alive(&shared, b), "trapping process B should survive");

    let msg = read_mailbox_tuple(&shared, b)
        .unwrap_or_else(|| panic!("B should have received EXIT message"));
    assert_eq!(msg.len(), 3, "EXIT message should be a 3-tuple");
    assert_eq!(msg[0], Term::atom(Atom::EXIT), "first element is EXIT atom");
    assert_eq!(msg[1].as_pid(), Some(1), "second element is dead PID");
    assert_eq!(msg[2], Term::atom(Atom::ERROR), "third element is reason");
}

#[test]
fn kill_propagates_killed_and_non_trapping_processes_die() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    let c = insert_process(&shared, 3);
    add_link(&shared, a, b);
    add_link(&shared, b, c);

    cleanup_exited_process(&shared, a, ExitReason::Kill);

    assert!(!is_alive(&shared, a), "process A removed");
    assert!(
        !is_alive(&shared, b),
        "B dies from killed signal (not trapping)"
    );
    assert!(!is_alive(&shared, c), "cascade kills C too");

    assert_eq!(
        shared.exit_tombstones.get(&b),
        Some(ExitReason::Killed),
        "B tombstone should be Killed"
    );
    assert_eq!(
        shared.exit_tombstones.get(&c),
        Some(ExitReason::Killed),
        "C tombstone should be Killed"
    );
}

#[test]
fn killed_signal_is_trappable_by_linked_process() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    add_link(&shared, a, b);
    set_trap_exit(&shared, b, true);

    cleanup_exited_process(&shared, a, ExitReason::Kill);

    assert!(!is_alive(&shared, a), "A removed");
    assert!(
        is_alive(&shared, b),
        "B traps exits and survives killed signal"
    );

    let msg =
        read_mailbox_tuple(&shared, b).unwrap_or_else(|| panic!("B should receive EXIT message"));
    assert_eq!(msg[0], Term::atom(Atom::EXIT));
    assert_eq!(msg[1].as_pid(), Some(1));
    assert_eq!(
        msg[2],
        Term::atom(Atom::KILLED),
        "reason is killed, not kill"
    );
}

#[test]
fn cascade_link_exit_propagates_through_chain() {
    let shared = make_shared_state();
    let a = insert_process(&shared, 1);
    let b = insert_process(&shared, 2);
    let c = insert_process(&shared, 3);
    let d = insert_process(&shared, 4);
    add_link(&shared, a, b);
    add_link(&shared, b, c);
    add_link(&shared, c, d);

    cleanup_exited_process(&shared, a, ExitReason::Error);

    assert!(!is_alive(&shared, a), "A removed");
    assert!(!is_alive(&shared, b), "B dies from link to A");
    assert!(!is_alive(&shared, c), "C dies from cascade via B");
    assert!(!is_alive(&shared, d), "D dies from cascade via C");
}

#[test]
fn cross_namespace_link_exit_propagates() {
    let shared = make_shared_state();
    let ns1 = NamespaceId(1);
    let ns2 = NamespaceId(2);
    let a = insert_process_in(&shared, 1, ns1);
    let b = insert_process_in(&shared, 2, ns2);
    add_link(&shared, a, b);

    cleanup_exited_process(&shared, a, ExitReason::Error);

    assert!(!is_alive(&shared, a), "source process removed");
    assert!(
        !is_alive(&shared, b),
        "linked process in another namespace dies from error exit"
    );
}
