use super::*;

fn plane() -> (ControlPlane, Arc<RecordingControlSender>) {
    let sender = Arc::new(RecordingControlSender::new());
    let plane = ControlPlane::new(Atom::OK, sender.clone());
    (plane, sender)
}

#[test]
fn decodes_monitor_control_opcodes() {
    assert_eq!(ControlOp::decode(19), Some(ControlOp::MonitorP));
    assert_eq!(ControlOp::decode(20), Some(ControlOp::DemonitorP));
    assert_eq!(ControlOp::decode(21), Some(ControlOp::MonitorPExit));
    assert_eq!(ControlOp::MonitorP.opcode(), 19);
    assert_eq!(ControlOp::DemonitorP.opcode(), 20);
    assert_eq!(ControlOp::MonitorPExit.opcode(), 21);
    assert_eq!(ControlOp::decode(0), None);
}

#[test]
fn monitor_remote_records_and_sends_monitor_p() {
    let (plane, sender) = plane();
    let target = RemotePid::new(Atom::ERROR, 42, 7);

    let reference = plane.monitor_remote(11, target).expect("monitor sends");

    assert!(reference >= REMOTE_MONITOR_REFERENCE_START);
    assert_eq!(
        sender.drain(),
        vec![OutboundControl {
            node: Atom::ERROR,
            message: ControlMessage::MonitorP {
                reference,
                watcher: RemotePid::new(Atom::OK, 11, 0),
                target,
            },
        }]
    );
}

#[test]
fn demonitor_remote_removes_record_and_sends_demonitor_p_once() {
    let (plane, sender) = plane();
    let target = RemotePid::new(Atom::ERROR, 42, 7);
    let reference = plane.monitor_remote(11, target).expect("monitor sends");
    sender.drain();

    assert!(
        plane
            .demonitor_remote(11, reference)
            .expect("demonitor sends")
    );
    assert_eq!(
        sender.drain(),
        vec![OutboundControl {
            node: Atom::ERROR,
            message: ControlMessage::DemonitorP {
                reference,
                watcher: RemotePid::new(Atom::OK, 11, 0),
                target,
            },
        }]
    );
    assert!(
        !plane
            .demonitor_remote(11, reference)
            .expect("idempotent miss")
    );
}

#[test]
fn demonitor_suppresses_later_monitor_p_exit_for_inbound_registration() {
    let (plane, sender) = plane();
    let watcher = RemotePid::new(Atom::ERROR, 11, 0);
    plane.register_inbound_monitor(5, watcher, 42);

    plane.remove_inbound_monitor(5, watcher, 42);
    let drained = plane.collect_inbound_for_target(42);

    assert!(drained.is_empty());
    assert!(sender.drain().is_empty());
}

#[test]
fn node_down_removes_inbound_watchers_for_failed_node() {
    let (plane, _sender) = plane();
    plane.register_inbound_monitor(5, RemotePid::new(Atom::ERROR, 11, 0), 42);
    plane.register_inbound_monitor(6, RemotePid::new(Atom::OK, 12, 0), 42);

    plane.remove_inbound_for_watcher_node(Atom::ERROR);

    assert_eq!(
        plane.collect_inbound_for_target(42),
        vec![InboundRemoteMonitor {
            watcher: RemotePid::new(Atom::OK, 12, 0),
            reference: 6,
            target_pid: 42,
        }]
    );
}
