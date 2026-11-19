//! DOWN messages through monitors, including to an executing watcher and
//! across namespaces.

use super::*;

#[test]
fn monitor_delivers_down_message_on_exit() {
    let shared = make_shared_state();
    let watcher = insert_process(&shared, 1);
    let target = insert_process(&shared, 2);
    let reference = add_monitor(&shared, watcher, target);

    cleanup_exited_process(&shared, target, ExitReason::Error);

    assert!(!is_alive(&shared, target), "target should be removed");
    assert!(is_alive(&shared, watcher), "watcher stays alive");

    let msg = read_mailbox_tuple(&shared, watcher)
        .unwrap_or_else(|| panic!("watcher should have received DOWN message"));
    assert_eq!(msg.len(), 5, "DOWN message should be a 5-tuple");
    assert_eq!(msg[0], Term::atom(Atom::DOWN), "first element is DOWN");
    let ref_term = boxed::Reference::new(msg[1])
        .unwrap_or_else(|| panic!("second element should be a boxed reference"));
    assert_eq!(ref_term.id(), reference, "reference matches");
    assert_eq!(
        msg[2],
        Term::atom(Atom::PROCESS),
        "third element is 'process'"
    );
    assert_eq!(msg[3].as_pid(), Some(2), "fourth element is dead PID");
    assert_eq!(msg[4], Term::atom(Atom::ERROR), "fifth element is reason");
}

#[test]
fn monitor_down_for_executing_watcher_is_delivered_on_store_back() {
    let shared = make_shared_state();
    let watcher = insert_process(&shared, 1);
    let target = insert_process(&shared, 2);
    let reference = add_monitor(&shared, watcher, target);
    let process = make_executing(&shared, watcher);

    cleanup_exited_process(&shared, target, ExitReason::Error);
    store_runnable_process(&shared, process);

    let msg = read_mailbox_tuple(&shared, watcher)
        .unwrap_or_else(|| panic!("executing watcher receives pending DOWN"));
    assert_eq!(msg.len(), 5, "DOWN message should be a 5-tuple");
    assert_eq!(msg[0], Term::atom(Atom::DOWN));
    let ref_term = boxed::Reference::new(msg[1]).unwrap_or_else(|| panic!("reference in DOWN"));
    assert_eq!(ref_term.id(), reference);
    assert_eq!(msg[2], Term::atom(Atom::PROCESS));
    assert_eq!(msg[3].as_pid(), Some(target));
    assert_eq!(msg[4], Term::atom(Atom::ERROR));
}

#[test]
fn monitor_and_link_both_fire_on_exit() {
    let shared = make_shared_state();
    let target = insert_process(&shared, 1);
    let linked = insert_process(&shared, 2);
    let watcher = insert_process(&shared, 3);
    add_link(&shared, target, linked);
    let reference = add_monitor(&shared, watcher, target);

    cleanup_exited_process(&shared, target, ExitReason::Error);

    assert!(!is_alive(&shared, target), "target removed");
    assert!(!is_alive(&shared, linked), "linked process dies");
    assert!(is_alive(&shared, watcher), "watcher stays alive");

    let msg =
        read_mailbox_tuple(&shared, watcher).unwrap_or_else(|| panic!("watcher receives DOWN"));
    assert_eq!(msg[0], Term::atom(Atom::DOWN));
    let ref_term = boxed::Reference::new(msg[1]).unwrap_or_else(|| panic!("reference in DOWN"));
    assert_eq!(ref_term.id(), reference);
}

#[test]
fn cross_namespace_monitor_delivers_down_message() {
    let shared = make_shared_state();
    let ns1 = NamespaceId(1);
    let ns2 = NamespaceId(2);
    let watcher = insert_process_in(&shared, 1, ns1);
    let target = insert_process_in(&shared, 2, ns2);
    let reference = add_monitor(&shared, watcher, target);

    cleanup_exited_process(&shared, target, ExitReason::Error);

    assert!(!is_alive(&shared, target), "target should be removed");
    assert!(is_alive(&shared, watcher), "watcher stays alive");
    let msg = read_mailbox_tuple(&shared, watcher)
        .unwrap_or_else(|| panic!("watcher should have received cross-namespace DOWN"));
    assert_eq!(msg[0], Term::atom(Atom::DOWN));
    let ref_term = boxed::Reference::new(msg[1]).unwrap_or_else(|| panic!("reference in DOWN"));
    assert_eq!(ref_term.id(), reference);
    assert_eq!(msg[3].as_pid(), Some(target));
    assert_eq!(msg[4], Term::atom(Atom::ERROR));
}
