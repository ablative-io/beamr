//! The control lane under pressure: exit storms, overflow to a healthy
//! peer, a parked writer, and a peer that never reads.

use super::*;

/// T-2 (DC-4, the R1 kill-shot): a 2000-process exit storm between two real
/// shared-state "nodes" on one socket pair — far past the 256-slot control
/// lane by construction — converges to EXACTLY one `{'EXIT', _, R}` per link,
/// each R ∈ {error, noconnection}: wire EXITs that rode generation G before
/// any overflow, and the noconnection backstop for everything after DC-1(b)
/// downed it. Nothing is lost, nothing is double-signalled.
#[test]
fn exit_storm_delivers_exactly_one_signal_per_link_across_nodes() {
    const STORM: u64 = 2000;
    const BASE: u64 = 1000;
    let shared_a = make_shared_state_with_dist_sender_named("a@test");
    let shared_b = make_shared_state_with_dist_sender_named("b@test");
    register_distribution_control_handler(&shared_b);
    register_scheduler_connection_subscriber(&shared_a);
    register_scheduler_connection_subscriber(&shared_b);

    let node_b_on_a = shared_a.atom_table.intern("b@test");
    let node_a_on_b = shared_b.atom_table.intern("a@test");
    let (server, client, addr) = socket_pair();
    {
        let handle = shared_a.dist_sender_or_panic().handle();
        let _context = handle.enter();
        shared_a
            .distribution_connections_or_panic()
            .register_test_connection(node_b_on_a, addr, server)
            .expect("register a->b connection");
    }
    {
        let handle = shared_b.dist_sender_or_panic().handle();
        let _context = handle.enter();
        shared_b
            .distribution_connections_or_panic()
            .register_test_connection(node_a_on_b, addr, client)
            .expect("register b->a connection");
    }

    for index in 0..STORM {
        let a_pid = insert_process(&shared_a, BASE + index);
        add_remote_link(
            &shared_a,
            a_pid,
            RemotePid {
                node: node_b_on_a,
                pid_number: BASE + index,
                serial: 0,
            },
        );
        let b_pid = insert_process(&shared_b, BASE + index);
        set_trap_exit(&shared_b, b_pid, true);
        add_remote_link(
            &shared_b,
            b_pid,
            RemotePid {
                node: node_a_on_b,
                pid_number: BASE + index,
                serial: 0,
            },
        );
    }

    // The burst. If the lane overflows mid-loop, the inline down-hook runs
    // right here on this thread and the remaining sends drop (no connection)
    // — their signals must arrive via B's own EOF backstop instead.
    for index in 0..STORM {
        cleanup_exited_process(&shared_a, BASE + index, ExitReason::Error);
    }

    // HS-5-style watchdog: convergence bounded by a deadline, not a sleep.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let total: usize = (0..STORM)
            .map(|index| mailbox_message_count(&shared_b, BASE + index))
            .sum();
        if total >= STORM as usize {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "exit storm never converged: {total}/{STORM} signals delivered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Settle briefly so a hypothetical late duplicate would be caught below.
    std::thread::sleep(Duration::from_millis(100));

    let mut wire_exits = 0_usize;
    let mut coarsened = 0_usize;
    for index in 0..STORM {
        let pid = BASE + index;
        assert_eq!(
            mailbox_message_count(&shared_b, pid),
            1,
            "exactly one exit signal per link (DC-4), pid {pid}"
        );
        let tuple = read_mailbox_tuple(&shared_b, pid).expect("exit tuple");
        assert_eq!(tuple.len(), 3);
        assert_eq!(tuple[0], Term::atom(Atom::EXIT));
        if tuple[2] == Term::atom(Atom::ERROR) {
            wire_exits += 1;
        } else if tuple[2] == Term::atom(Atom::NOCONNECTION) {
            coarsened += 1;
        } else {
            panic!("unexpected exit reason {:?} for pid {pid}", tuple[2]);
        }
        assert!(is_alive(&shared_b, pid), "trapping target survives");
    }
    assert_eq!(wire_exits + coarsened, STORM as usize);
    if coarsened > 0 {
        // Overflow path: the pinned generation is down on A immediately, and
        // B converges via EOF within a bounded window (DC-1 both-sides).
        assert!(
            shared_a
                .distribution_connections_or_panic()
                .get_connection(node_b_on_a)
                .is_none(),
            "overflow must purge the pinned connection on the sending side"
        );
        let eof_deadline = Instant::now() + Duration::from_secs(30);
        while shared_b
            .distribution_connections_or_panic()
            .get_connection(node_a_on_b)
            .is_some()
        {
            assert!(
                Instant::now() < eof_deadline,
                "B never observed the downed pair via EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// T-8 (ruling 8, the accepted cross-peer blast radius, pinned): with the
/// shared control lane wedged full behind a never-reading peer, a control to
/// a HEALTHY peer overflows and downs the healthy pin — and the inline hook
/// converts the caller's links to `noconnection` before the facility call
/// returns. Correctness holds (no lost signal, DC-1/DC-3); the healthy pair
/// is merely down pending redial — an availability blip.
///
/// The lane is filled behind the parked write with controls pinned to a THIRD
/// connection, so the first overflow downs that filler pin — NOT the parked
/// writer. (Downing the parked writer shuts its socket down and unparks the
/// drain — the commit-4 improvement pinned by the next test — which would
/// dissolve the full-lane condition this test exists to exercise.)
#[test]
fn control_overflow_to_healthy_peer_converges_to_noconnection_blip() {
    let shared = make_shared_state_with_dist_sender();
    register_scheduler_connection_subscriber(&shared);
    let sender = shared.dist_sender_or_panic();
    let handle = sender.handle();
    let _context = handle.enter();

    let wedged_node = shared.atom_table.intern("wedged@test");
    let healthy_node = shared.atom_table.intern("healthy@test");
    let filler_node = shared.atom_table.intern("filler@test");
    // Held but NEVER read: writes to it park once the kernel buffers fill.
    let wedged_peer = install_peer(&shared, wedged_node);
    let _healthy_peer = install_peer(&shared, healthy_node);
    let _filler_peer = install_peer(&shared, filler_node);
    let wedged = shared
        .distribution_connections_or_panic()
        .get_connection(wedged_node)
        .expect("wedged connection is in the table");
    let filler_pin = shared
        .distribution_connections_or_panic()
        .get_connection(filler_node)
        .expect("filler connection is in the table");

    let watcher = insert_process(&shared, 1);
    set_trap_exit(&shared, watcher, true);
    let healthy_remote = RemotePid {
        node: healthy_node,
        pid_number: 9,
        serial: 0,
    };
    add_remote_link(&shared, watcher, healthy_remote);

    // Park the drain: one oversized control to the never-reading peer blocks
    // `write_all` until WRITE_TIMEOUT (5 s), and the lane fills behind it.
    let mut big = vec![0_u8; 16 * 1024 * 1024];
    big[0] = 1;
    let control_len = u32::try_from(big.len()).expect("control fits u32");
    let mut big_frame = Vec::with_capacity(8 + big.len());
    big_frame.extend_from_slice(&control_len.to_be_bytes());
    big_frame.extend_from_slice(&0_u32.to_be_bytes());
    big_frame.extend_from_slice(&big);
    sender
        .enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged),
            frame: Arc::from(big_frame.into_boxed_slice()),
        })
        .expect("first control accepted into an empty lane");
    // The drain has begun (and therefore parked on) the oversized write once
    // the peer side observes its first byte — everything below happens well
    // inside the 5 s window during which the drain frees no lane slot.
    wedged_peer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("peer read timeout");
    let mut probe = [0_u8; 1];
    assert_eq!(
        wedged_peer
            .peek(&mut probe)
            .expect("drain begins the wedged write"),
        1
    );

    // Fill the lane behind the parked write with FILLER-pinned controls. The
    // first Overflow marks the FILLER pin down (its own DC-1(b)) and proves
    // the lane is full — while the wedged pin stays up, so its parked write
    // keeps the drain parked and the lane stays full.
    let filler: Arc<[u8]> = Arc::from(vec![0_u8; 8].into_boxed_slice());
    let mut overflowed = false;
    for _ in 0..=crate::distribution::sender::DIST_CONTROL_QUEUE_CAP {
        match sender.enqueue_control(ControlOutbound {
            connection: Arc::clone(&filler_pin),
            frame: Arc::clone(&filler),
        }) {
            Ok(()) => {}
            Err(_) => {
                overflowed = true;
                break;
            }
        }
    }
    assert!(
        overflowed,
        "filling behind a parked write must fill the lane"
    );
    assert!(
        filler_pin.is_down(),
        "overflow downs its own pinned connection"
    );
    assert!(
        !wedged.is_down(),
        "the parked writer stays up, keeping the drain parked"
    );
    assert_eq!(
        mailbox_message_count(&shared, watcher),
        0,
        "the healthy link is untouched so far"
    );

    // The accepted-risk moment: a control to the HEALTHY peer hits the full
    // shared lane. INV-SYNC on the hook chain means the noconnection is in
    // the watcher's mailbox before exit_remote returns.
    let facility = control_facility(&shared);
    assert_eq!(
        facility.exit_remote(watcher, healthy_remote, ExitReason::Error),
        Ok(()),
        "EXIT2 stays best-effort Ok even when it overflows"
    );

    let tuple = read_mailbox_tuple(&shared, watcher).expect("noconnection EXIT");
    assert_noconnection_exit(&tuple, healthy_remote);
    assert_eq!(
        mailbox_message_count(&shared, watcher),
        1,
        "blip, not a storm"
    );
    assert!(is_alive(&shared, watcher), "trapping watcher survives");
    assert!(
        shared
            .distribution_connections_or_panic()
            .get_connection(healthy_node)
            .is_none(),
        "the healthy pair is down pending redial"
    );
}

/// The commit-4 shrink of T-8's blast radius, pinned: downing the connection
/// whose write is PARKED (here via its own lane overflow) shuts the socket
/// down at `mark_down`, so the parked write errors IMMEDIATELY — the drain
/// recovers well inside WRITE_TIMEOUT and a subsequent control to a healthy
/// peer is WRITTEN, not overflowed into a spurious noconnection. Pre-commit-4
/// the drain stayed parked for the full 5 s WRITE_TIMEOUT and the healthy
/// peer took the blip.
#[test]
fn downing_the_parked_writer_unparks_the_drain_before_write_timeout() {
    use std::io::Read;

    let shared = make_shared_state_with_dist_sender();
    register_scheduler_connection_subscriber(&shared);
    let sender = shared.dist_sender_or_panic();
    let handle = sender.handle();
    let _context = handle.enter();

    let wedged_node = shared.atom_table.intern("wedged2@test");
    let healthy_node = shared.atom_table.intern("healthy2@test");
    let mut wedged_peer = install_peer(&shared, wedged_node);
    let mut healthy_peer = install_peer(&shared, healthy_node);
    let wedged = shared
        .distribution_connections_or_panic()
        .get_connection(wedged_node)
        .expect("wedged connection is in the table");

    let watcher = insert_process(&shared, 1);
    set_trap_exit(&shared, watcher, true);
    let healthy_remote = RemotePid {
        node: healthy_node,
        pid_number: 9,
        serial: 0,
    };
    add_remote_link(&shared, watcher, healthy_remote);

    // Park the drain on an oversized write to the never-reading peer.
    let mut big = vec![0_u8; 16 * 1024 * 1024];
    big[0] = 1;
    let control_len = u32::try_from(big.len()).expect("control fits u32");
    let mut big_frame = Vec::with_capacity(8 + big.len());
    big_frame.extend_from_slice(&control_len.to_be_bytes());
    big_frame.extend_from_slice(&0_u32.to_be_bytes());
    big_frame.extend_from_slice(&big);
    sender
        .enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged),
            frame: Arc::from(big_frame.into_boxed_slice()),
        })
        .expect("first control accepted into an empty lane");
    wedged_peer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("peer read timeout");
    let mut probe = [0_u8; 1];
    assert_eq!(
        wedged_peer
            .peek(&mut probe)
            .expect("drain begins the wedged write"),
        1
    );

    // Fill the lane with WEDGED-pinned fillers: the first overflow downs the
    // wedged pin itself — whose mark_down shuts the socket down, erroring the
    // parked write.
    let filler: Arc<[u8]> = Arc::from(vec![0_u8; 8].into_boxed_slice());
    let mut overflowed = false;
    for _ in 0..=crate::distribution::sender::DIST_CONTROL_QUEUE_CAP {
        match sender.enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged),
            frame: Arc::clone(&filler),
        }) {
            Ok(()) => {}
            Err(_) => {
                overflowed = true;
                break;
            }
        }
    }
    assert!(overflowed, "filling behind the parked write fills the lane");
    assert!(wedged.is_down(), "overflow downs its own pinned connection");

    // The parked write errored: the wedged peer observes EOF/RST after
    // draining whatever landed in its kernel buffer — promptly, not at
    // WRITE_TIMEOUT.
    let mut sink = [0_u8; 64 * 1024];
    loop {
        match wedged_peer.read(&mut sink) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("wedged peer expected EOF, read failed: {error}"),
        }
    }
    // Give the recovered drain a beat to skip the down-pinned fillers (a
    // handful of microseconds of is_down checks; the margin is host noise).
    std::thread::sleep(Duration::from_millis(200));

    // The drain has recovered: an EXIT2 to the healthy peer is WRITTEN — no
    // overflow, no spurious noconnection, connection intact.
    let facility = control_facility(&shared);
    assert_eq!(
        facility.exit_remote(watcher, healthy_remote, ExitReason::Error),
        Ok(())
    );
    healthy_peer
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("healthy read timeout");
    let mut received = [0_u8; 8];
    assert!(
        healthy_peer
            .read(&mut received)
            .expect("EXIT2 frame arrives")
            > 0,
        "the healthy peer receives the control on the wire"
    );
    assert_eq!(
        mailbox_message_count(&shared, watcher),
        0,
        "no spurious noconnection for the healthy link"
    );
    assert!(
        shared
            .distribution_connections_or_panic()
            .get_connection(healthy_node)
            .is_some(),
        "the healthy connection survives"
    );
}

/// Round-3 major 1: the DIRECT (synchronous) remote-send write is bounded by
/// the same deadline class the drain uses. A peer that is TCP-connected but
/// never reads would otherwise park the calling scheduler thread indefinitely
/// — no other machinery ends that wait (the heartbeat's own writes queue
/// behind the same writer mutex, and inbound keepalives keep inbound liveness
/// fresh while outbound is wedged). On elapse the connection is retired via
/// mark_down_write_timeout.
#[test]
fn direct_send_to_a_never_reading_peer_is_bounded_and_retires_the_connection() {
    let shared = make_shared_state_with_dist_sender();
    register_scheduler_connection_subscriber(&shared);
    let sender = shared.dist_sender_or_panic();
    let handle = sender.handle();
    let _context = handle.enter();

    let wedged_node = shared.atom_table.intern("wedged3@test");
    // Held but NEVER read: an oversized write parks once kernel buffers fill.
    let _wedged_peer = install_peer(&shared, wedged_node);

    let frame = vec![0_u8; 16 * 1024 * 1024];
    let started = Instant::now();
    let result = supervision_integration::block_on_distribution_send_with_write_deadline(
        shared.distribution_connections_or_panic(),
        wedged_node,
        "wedged3@test",
        &frame,
        Duration::from_millis(200),
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the direct send must return within its deadline class, not hang"
    );
    assert_eq!(
        result,
        Err(crate::distribution::control::DistributionSendError::NoConnection),
        "a timed-out direct send reports no-connection"
    );
    assert!(
        shared
            .distribution_connections_or_panic()
            .get_connection(wedged_node)
            .is_none(),
        "the write timeout retires the wedged connection"
    );
}
