use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use super::*;
use crate::atom::AtomTable;
use crate::distribution::connection::{ConnectionDownReason, ConnectionManager};
use crate::distribution::etf::MAX_DIST_FRAME_BYTES;
use crate::distribution::resolver::StaticResolver;

fn manager() -> (ConnectionManager, Arc<AtomTable>) {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let resolver = Arc::new(StaticResolver::new(HashMap::new()));
    (
        ConnectionManager::new(
            Arc::clone(&atom_table),
            resolver,
            "test-cookie",
            "local@test",
            0,
        ),
        atom_table,
    )
}

/// A length-prefixed frame the read-lifecycle parser accepts: 8-byte header
/// (control_len, payload_len) followed by `control_len + payload_len` bytes.
fn framed(control: &[u8]) -> Arc<[u8]> {
    let control_len = u32::try_from(control.len()).expect("control fits u32");
    let mut frame = Vec::with_capacity(8 + control.len());
    frame.extend_from_slice(&control_len.to_be_bytes());
    frame.extend_from_slice(&0u32.to_be_bytes());
    frame.extend_from_slice(control);
    Arc::from(frame.into_boxed_slice())
}

/// Enqueue never blocks and silently drops once the bounded queue is full,
/// even with no peer connected and the drain unable to make progress.
#[test]
fn enqueue_is_non_blocking_and_drops_when_full() {
    let (connections, atom_table) = manager();
    let sender = DistSender::new(connections).expect("sender builds");
    let node = atom_table.intern("absent@127.0.0.1");

    // Far more than the queue capacity. With no connection the drain drops
    // each item, but even if it stalled, `enqueue` must return promptly for
    // every call and never panic.
    for index in 0..(DIST_SEND_QUEUE_CAP * 4) {
        sender.enqueue(DistOutbound::ToNode {
            node,
            frame: framed(&index.to_be_bytes()),
        });
    }
    sender.shutdown();
}

/// Frames enqueued for one node arrive at that node in FIFO order: the single
/// drain plus the per-connection writer `Mutex` serialise writes.
///
/// Single-threaded `#[tokio::test]` deliberately: it also exercises FIX 1 by
/// letting the owned `DistSender` drop directly inside this async context. The
/// `DistSenderInner::Drop` impl moves the blocking runtime shutdown onto a
/// dedicated `std::thread`, so the drop must NOT panic even here, where there
/// is no `block_in_place` escape hatch and a naive runtime drop would abort.
#[tokio::test]
async fn per_node_fifo_ordering() {
    let (connections, atom_table) = manager();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    // Peer side: read every frame and record its 1-byte control sequence id.
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_for_task = Arc::clone(&received);
    let count = 16usize;
    let reader = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        for _ in 0..count {
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_err() {
                break;
            }
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_err() {
                break;
            }
            received_for_task
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(body[0]);
        }
    });

    // Register the accepted client stream as a connection on the manager.
    let std_stream = std::net::TcpStream::connect(addr).expect("client connects");
    let node = atom_table.intern("peer@127.0.0.1");
    let peer_addr: SocketAddr = std_stream.peer_addr().expect("peer addr");
    connections
        .register_test_connection(node, peer_addr, std_stream)
        .expect("register test connection");

    let sender = DistSender::new(connections).expect("sender builds");
    for index in 0..count {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender.enqueue(DistOutbound::ToNode {
            node,
            frame: framed(&[seq]),
        });
    }

    reader.await.expect("reader task joins");
    let order = received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let expected: Vec<u8> = (0..count).map(|i| i as u8).collect();
    assert_eq!(order, expected, "frames must arrive in enqueue order");
    sender.shutdown();
    // Drop directly in the async context. FIX 1 (the dedicated-thread runtime
    // drop) is what keeps this from panicking — see the test doc comment.
    drop(sender);
}

/// A dead peer (write half closed) does not stall the drain: a second, live
/// node still receives its frame, and the dead connection's down-hook fires.
///
/// Single-threaded `#[tokio::test]`, like `per_node_fifo_ordering`, so the
/// final direct `drop(sender)` also proves FIX 1's async-safe runtime drop.
#[tokio::test]
async fn dead_peer_does_not_stall_drain() {
    let (connections, atom_table) = manager();
    let down_count = Arc::new(AtomicUsize::new(0));
    let down_for_hook = Arc::clone(&down_count);
    connections.register_connection_down(move |_| {
        down_for_hook.fetch_add(1, Ordering::SeqCst);
    });

    // Dead node: connect a stream, then drop the peer's read half so writes
    // eventually fail and mark it down.
    let dead_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind dead");
    let dead_addr = dead_listener.local_addr().expect("dead addr");
    let dead_node = atom_table.intern("dead@127.0.0.1");
    let dead_stream = std::net::TcpStream::connect(dead_addr).expect("dead connects");
    let dead_peer_addr = dead_stream.peer_addr().expect("dead peer addr");
    let dead_accept = tokio::spawn(async move { dead_listener.accept().await });
    connections
        .register_test_connection(dead_node, dead_peer_addr, dead_stream)
        .expect("register dead connection");
    let accepted = dead_accept
        .await
        .expect("dead accept join")
        .expect("accepted");
    drop(accepted); // close the peer so writes to `dead_node` fail.

    // Live node: a real reader that records what it receives.
    let live_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live");
    let live_addr = live_listener.local_addr().expect("live addr");
    let live_received = Arc::new(Mutex::new(Vec::new()));
    let live_for_task = Arc::clone(&live_received);
    let live_reader = tokio::spawn(async move {
        let (mut stream, _) = live_listener.accept().await.expect("live accept");
        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).await.is_ok() {
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_ok() {
                live_for_task
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(body[0]);
            }
        }
    });
    let live_stream = std::net::TcpStream::connect(live_addr).expect("live connects");
    let live_node = atom_table.intern("live@127.0.0.1");
    let live_peer_addr = live_stream.peer_addr().expect("live peer addr");
    connections
        .register_test_connection(live_node, live_peer_addr, live_stream)
        .expect("register live connection");

    let sender = DistSender::new(connections.clone()).expect("sender builds");
    // Many frames to the dead node first, then one to the live node. If the
    // drain stalled on the dead peer, the live frame would never arrive.
    for index in 0..32u8 {
        sender.enqueue(DistOutbound::ToNode {
            node: dead_node,
            frame: framed(&[index]),
        });
    }
    sender.enqueue(DistOutbound::ToNode {
        node: live_node,
        frame: framed(&[0xAB]),
    });

    // The live reader joining proves the drain made progress past the dead
    // peer (bounded by its read-exact, not a fixed sleep).
    live_reader.await.expect("live reader joins");
    let got = live_received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(got, vec![0xAB], "live node must still receive its frame");

    // The dead connection's down-hook must have fired (write failure path).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while down_count.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "dead peer down-hook never fired"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(connections.get_connection(dead_node).is_none());
    sender.shutdown();
    // Drop directly in the async context; FIX 1 keeps it panic-free.
    drop(sender);
}

/// FIX 3: a peer that is TCP-connected but never reads (its kernel send
/// buffer fills, so `write_all` would otherwise park ~2h until OS keepalive)
/// must NOT stall the shared drain indefinitely. The per-write [`WRITE_TIMEOUT`]
/// turns the wedged write into a write failure: the wedged connection is
/// marked down (down-hook fires, connection purged) and the drain proceeds to
/// the healthy peer, which still receives its frame — bounded by the timeout,
/// not the kernel keepalive.
///
/// Multi-threaded so the wedged blocking write and the test's own polling can
/// make progress concurrently on the test runtime; the `DistSender` has its
/// own runtime regardless.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wedged_peer_does_not_stall_drain_indefinitely() {
    let (connections, atom_table) = manager();
    let down_count = Arc::new(AtomicUsize::new(0));
    let down_for_hook = Arc::clone(&down_count);
    connections.register_connection_down(move |_| {
        down_for_hook.fetch_add(1, Ordering::SeqCst);
    });

    // Wedged node: accept the connection but NEVER read from it. Holding the
    // accepted stream (without reading) keeps the peer connected, so writes do
    // not fail — they block once the kernel send+recv buffers fill.
    let wedged_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind wedged");
    let wedged_addr = wedged_listener.local_addr().expect("wedged addr");
    let wedged_node = atom_table.intern("wedged@127.0.0.1");
    let wedged_stream = std::net::TcpStream::connect(wedged_addr).expect("wedged connects");
    let wedged_peer_addr = wedged_stream.peer_addr().expect("wedged peer addr");
    let wedged_accept = tokio::spawn(async move { wedged_listener.accept().await });
    connections
        .register_test_connection(wedged_node, wedged_peer_addr, wedged_stream)
        .expect("register wedged connection");
    let wedged_accepted = wedged_accept
        .await
        .expect("wedged accept join")
        .expect("wedged accepted");
    // Keep the accepted half alive but never read it: this is what wedges the
    // writer. Dropping it would instead fail the write fast (the dead-peer
    // case, already covered separately).
    let _wedged_held = wedged_accepted;

    // Healthy node: a real reader that records what it receives.
    let live_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live");
    let live_addr = live_listener.local_addr().expect("live addr");
    let live_received = Arc::new(Mutex::new(Vec::new()));
    let live_for_task = Arc::clone(&live_received);
    let live_reader = tokio::spawn(async move {
        let (mut stream, _) = live_listener.accept().await.expect("live accept");
        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).await.is_ok() {
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_ok() {
                live_for_task
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(body[0]);
            }
        }
    });
    let live_stream = std::net::TcpStream::connect(live_addr).expect("live connects");
    let live_node = atom_table.intern("live@127.0.0.1");
    let live_peer_addr = live_stream.peer_addr().expect("live peer addr");
    connections
        .register_test_connection(live_node, live_peer_addr, live_stream)
        .expect("register live connection");

    let sender = DistSender::new(connections.clone()).expect("sender builds");

    // One frame to the wedged node large enough to overflow the kernel send
    // and receive buffers (which the peer never drains), so `write_all` parks
    // and only `WRITE_TIMEOUT` can release it. 16 MiB exceeds default socket
    // buffers on Linux and macOS by orders of magnitude.
    let mut big = vec![0u8; 16 * 1024 * 1024];
    big[0] = 0x01;
    let big_control_len = u32::try_from(big.len()).expect("control fits u32");
    let mut wedged_frame = Vec::with_capacity(8 + big.len());
    wedged_frame.extend_from_slice(&big_control_len.to_be_bytes());
    wedged_frame.extend_from_slice(&0u32.to_be_bytes());
    wedged_frame.extend_from_slice(&big);
    sender.enqueue(DistOutbound::ToNode {
        node: wedged_node,
        frame: Arc::from(wedged_frame.into_boxed_slice()),
    });
    // Then a small frame to the healthy node, enqueued AFTER the wedged one.
    // The single drain reaches it only once the wedged write is released by
    // the timeout — proving the stall is bounded, not indefinite.
    sender.enqueue(DistOutbound::ToNode {
        node: live_node,
        frame: framed(&[0xAB]),
    });

    // The healthy reader must still join — but only after the wedged write
    // times out. Bound the wait generously above WRITE_TIMEOUT (5s) so the
    // test proves "bounded by the timeout" without flaking, yet would fail
    // hard on an indefinite (~2h keepalive) stall.
    let live_join = tokio::time::timeout(Duration::from_secs(30), live_reader)
        .await
        .expect("healthy peer received within the bounded window (not a ~2h stall)");
    live_join.expect("live reader task joins");
    let got = live_received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(got, vec![0xAB], "healthy node must still receive its frame");

    // The wedged connection must have been marked down via the write-timeout
    // path (down-hook fired, connection purged from the table).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while down_count.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "wedged peer down-hook never fired after the write timeout"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        connections.get_connection(wedged_node).is_none(),
        "wedged connection must be purged after the write timeout"
    );

    sender.shutdown();
    drop(sender);
}

/// DC-5: control frames enqueued against one pinned connection arrive at
/// that node in FIFO order — one bounded lane, one drain task, one
/// per-connection writer `Mutex`. Mirror of `per_node_fifo_ordering` on the
/// control lane.
#[tokio::test]
async fn control_lane_per_node_fifo_ordering() {
    let (connections, atom_table) = manager();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    // Peer side: read every frame and record its 1-byte control sequence id.
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_for_task = Arc::clone(&received);
    let count = 16usize;
    let reader = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        for _ in 0..count {
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_err() {
                break;
            }
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_err() {
                break;
            }
            received_for_task
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(body[0]);
        }
    });

    let std_stream = std::net::TcpStream::connect(addr).expect("client connects");
    let node = atom_table.intern("peer@127.0.0.1");
    let peer_addr: SocketAddr = std_stream.peer_addr().expect("peer addr");
    let connection = connections
        .register_test_connection(node, peer_addr, std_stream)
        .expect("register test connection");

    let sender = DistSender::new(connections).expect("sender builds");
    for index in 0..count {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender
            .enqueue_control(ControlOutbound {
                connection: Arc::clone(&connection),
                frame: framed(&[seq]),
            })
            .expect("control lane accepts below capacity");
    }

    reader.await.expect("reader task joins");
    let order = received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let expected: Vec<u8> = (0..count).map(|i| i as u8).collect();
    assert_eq!(order, expected, "controls must arrive in enqueue order");
    sender.shutdown();
    drop(sender);
}

/// T-1 (DC-1): flooding the control lane against a wedged peer (accepted
/// but never read, so the drain parks on the first oversized write) must
/// (a) never block the caller, (b) overflow and mark the PINNED connection
/// down exactly once — `ControlOverflow` at enqueue, or `WriteTimeout` if
/// the write deadline won the race; either is DC-1(b) — and (c) purge the
/// connection from the table.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_lane_overflow_flood_marks_wedged_peer_down_exactly_once() {
    let (connections, atom_table) = manager();
    let down_reasons = Arc::new(Mutex::new(Vec::new()));
    let down_for_hook = Arc::clone(&down_reasons);
    connections.register_connection_down(move |event| {
        down_for_hook
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event.reason);
    });

    // Wedged node: accept the connection but NEVER read from it, so writes
    // block once the kernel send+recv buffers fill (see
    // `wedged_peer_does_not_stall_drain_indefinitely`).
    let wedged_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind wedged");
    let wedged_addr = wedged_listener.local_addr().expect("wedged addr");
    let wedged_node = atom_table.intern("wedged@127.0.0.1");
    let wedged_stream = std::net::TcpStream::connect(wedged_addr).expect("wedged connects");
    let wedged_peer_addr = wedged_stream.peer_addr().expect("wedged peer addr");
    let wedged_accept = tokio::spawn(async move { wedged_listener.accept().await });
    let wedged_connection = connections
        .register_test_connection(wedged_node, wedged_peer_addr, wedged_stream)
        .expect("register wedged connection");
    let _wedged_held = wedged_accept
        .await
        .expect("wedged accept join")
        .expect("wedged accepted");

    let sender = DistSender::new(connections.clone()).expect("sender builds");

    // Park the drain: one control frame large enough to overflow the kernel
    // buffers the peer never drains, so `write_all` blocks until
    // WRITE_TIMEOUT and the lane fills behind it.
    let mut big = vec![0u8; 16 * 1024 * 1024];
    big[0] = 0x01;
    let big_control_len = u32::try_from(big.len()).expect("control fits u32");
    let mut big_frame = Vec::with_capacity(8 + big.len());
    big_frame.extend_from_slice(&big_control_len.to_be_bytes());
    big_frame.extend_from_slice(&0u32.to_be_bytes());
    big_frame.extend_from_slice(&big);
    sender
        .enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged_connection),
            frame: Arc::from(big_frame.into_boxed_slice()),
        })
        .expect("first control accepted into an empty lane");

    // Flood far past capacity. Every call must return promptly (try_send,
    // never a blocking wait on the wedged write) and the lane must report
    // Overflow once full — having ALREADY marked the pinned connection down.
    let start = std::time::Instant::now();
    let mut overflowed = 0usize;
    for index in 0..(DIST_CONTROL_QUEUE_CAP * 4) {
        match sender.enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged_connection),
            frame: framed(&index.to_be_bytes()),
        }) {
            Ok(()) => {}
            Err(ControlEnqueueError::Overflow) => {
                // The Overflow contract: the pinned connection was ALREADY
                // marked down before enqueue_control returned.
                assert!(
                    wedged_connection.is_down(),
                    "Overflow must mark the pinned connection down before returning"
                );
                overflowed += 1;
            }
            Err(ControlEnqueueError::Closed) => {
                panic!("control lane must not close while the sender is live")
            }
        }
    }
    let elapsed = start.elapsed();
    assert!(
        overflowed > 0,
        "flooding 4x capacity behind a wedged write must overflow the lane"
    );
    assert!(
        elapsed < WRITE_TIMEOUT,
        "enqueue_control must be non-blocking; flood took {elapsed:?}"
    );

    // The down-hook fires exactly once (mark_down is once-guarded), with a
    // DC-1(b) reason, and the connection leaves the table.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while down_reasons
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_empty()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "overflow down-hook never fired"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let reasons = down_reasons
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(
        reasons.len(),
        1,
        "down-hook must fire exactly once, got {reasons:?}"
    );
    assert!(
        matches!(
            reasons[0],
            ConnectionDownReason::ControlOverflow | ConnectionDownReason::WriteTimeout
        ),
        "down reason must be a DC-1(b) arm, got {:?}",
        reasons[0]
    );
    assert!(wedged_connection.is_down());
    assert!(
        connections.get_connection(wedged_node).is_none(),
        "wedged connection must be purged from the table"
    );

    sender.shutdown();
    drop(sender);
}

/// T-3 (DC-2 generation pinning): a control enqueued against a downed
/// generation G is skipped by the drain (`is_down`), never re-resolved by
/// node onto the post-redial socket. The biased drain handles the control
/// lane first, so if pinning were broken the stale control — not the
/// data-lane sentinel — would be the first frame on the new socket.
#[tokio::test]
async fn control_frame_pinned_to_down_generation_never_reaches_redialed_socket() {
    let (connections, atom_table) = manager();
    let node = atom_table.intern("peer@127.0.0.1");

    // Generation G: register, hold the peer half alive, capture the Arc.
    let listener_g = TcpListener::bind("127.0.0.1:0").await.expect("bind G");
    let addr_g = listener_g.local_addr().expect("G addr");
    let stream_g = std::net::TcpStream::connect(addr_g).expect("G connects");
    let peer_addr_g = stream_g.peer_addr().expect("G peer addr");
    let accept_g = tokio::spawn(async move { listener_g.accept().await });
    let pinned = connections
        .register_test_connection(node, peer_addr_g, stream_g)
        .expect("register generation G");
    let _held_g = accept_g.await.expect("G accept join").expect("G accepted");

    // Down G, then "redial": a fresh connection takes the node key.
    pinned.mark_down_write_timeout();
    assert!(pinned.is_down());
    assert!(
        connections.get_connection(node).is_none(),
        "downed generation must leave the table before the redial"
    );

    let listener_new = TcpListener::bind("127.0.0.1:0").await.expect("bind new");
    let addr_new = listener_new.local_addr().expect("new addr");
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_for_task = Arc::clone(&received);
    let reader = tokio::spawn(async move {
        let (mut stream, _) = listener_new.accept().await.expect("new accept");
        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).await.is_ok() {
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_ok() {
                received_for_task
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(body[0]);
            }
        }
    });
    let stream_new = std::net::TcpStream::connect(addr_new).expect("new connects");
    let peer_addr_new = stream_new.peer_addr().expect("new peer addr");
    connections
        .register_test_connection(node, peer_addr_new, stream_new)
        .expect("register redialed connection");

    let sender = DistSender::new(connections.clone()).expect("sender builds");
    // Control pinned to the DOWN generation G: accepted into the lane, but
    // the drain must skip it rather than resolve `node` to the new socket.
    sender
        .enqueue_control(ControlOutbound {
            connection: Arc::clone(&pinned),
            frame: framed(&[0xCC]),
        })
        .expect("enqueue accepts; the is_down skip happens at the drain");
    // Data-lane sentinel resolved by node to the NEW connection, enqueued
    // AFTER the control (which the biased drain also prefers).
    sender.enqueue(DistOutbound::ToNode {
        node,
        frame: framed(&[0xDD]),
    });

    reader.await.expect("reader task joins");
    let got = received
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(
        got,
        vec![0xDD],
        "first frame on the redialed socket must be the sentinel, never the pinned control"
    );
    sender.shutdown();
    drop(sender);
}

#[test]
fn disconnect_all_closes_a_connection_whose_writer_is_wedged() {
    // Round-2 major 1: closure must NOT depend on the writer mutex. A huge
    // write to a peer that never reads blocks holding the mutex;
    // `disconnect_all`'s socket-level shutdown(2) on the owned dup must
    // error that write out promptly and FIN the peer anyway. Pre-fix, the
    // try_lock close is skipped and the write blocks forever.
    use std::io::Read;

    let (manager, atom_table) = manager();
    let sender = DistSender::new(manager.clone()).expect("sender builds");
    manager.set_runtime_handle(sender.handle());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let mut client = std::net::TcpStream::connect(addr).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    let node = atom_table.intern("peer@wedged");
    let connection = {
        let handle = sender.handle();
        let _context = handle.enter();
        manager
            .register_test_connection(node, addr, server)
            .expect("register test connection")
    };

    let (write_tx, write_rx) = std::sync::mpsc::channel();
    let write_connection = Arc::clone(&connection);
    sender.handle().spawn(async move {
        // Far beyond any default kernel buffer, and the peer never reads:
        // this write blocks mid-poll holding the writer mutex.
        let payload = vec![0u8; 16 * 1024 * 1024];
        let result = write_connection.write_raw(&payload).await;
        let _ = write_tx.send(result.is_err());
    });
    // Let the write reach its blocked state before teardown contends.
    std::thread::sleep(Duration::from_millis(300));

    manager.disconnect_all();

    assert!(
        write_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the wedged write must finish once the socket is shut down"),
        "the wedged write reports an error after socket shutdown"
    );
    assert!(manager.connected_nodes().is_empty());
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let mut buffer = [0u8; 64];
    loop {
        // Drain whatever landed before the shutdown; EOF/reset is the pin.
        match client.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("peer expected EOF, read failed: {error}"),
        }
    }
    sender.shutdown();
}

#[test]
fn worker_side_inventory_during_shutdown_does_not_deadlock() {
    // Round-2 major 2: shutdown must not hold the runtime mutex across the
    // blocking join. A worker-side task reading `worker_thread_names()`
    // (the inventory path) while shutdown waits for that worker was a
    // lock-inversion deadlock.
    let (manager, _atom_table) = manager();
    let sender = DistSender::new(manager).expect("sender builds");
    let probe = sender.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (names_tx, names_rx) = std::sync::mpsc::channel();
    sender.handle().spawn(async move {
        let _ = started_tx.send(());
        // Synchronous sleep: keeps the worker busy mid-poll while the
        // shutdown thread takes the runtime slot, so the names read below
        // contends with the join, not with the take.
        std::thread::sleep(Duration::from_millis(100));
        let _ = names_tx.send(probe.worker_thread_names());
    });
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker task starts");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let owner = sender.clone();
    let shutdown_thread = std::thread::spawn(move || {
        owner.shutdown();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("shutdown must not deadlock against a worker-side inventory read");
    let _ = shutdown_thread.join();
    names_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker-side inventory read completes");
}

#[test]
fn shutdown_from_an_unrelated_runtime_still_joins_the_worker() {
    // Round-2 major 3: only the TRUE self-runtime case may fall back to a
    // non-joining teardown. From an unrelated tokio runtime, shutdown must
    // remain join-complete — pinned by time: the worker is deliberately
    // busy in a synchronous sleep, so a joined shutdown cannot return
    // before that sleep finishes, while a background fallback returns
    // immediately.
    let (manager, _atom_table) = manager();
    let sender = DistSender::new(manager).expect("sender builds");
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    sender.handle().spawn(async move {
        let _ = busy_tx.send(());
        std::thread::sleep(Duration::from_millis(1200));
    });
    busy_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("busy task starts");

    let unrelated = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("unrelated runtime builds");
    let for_shutdown = sender.clone();
    let started = std::time::Instant::now();
    unrelated.block_on(async move {
        for_shutdown.shutdown();
    });
    assert!(
        started.elapsed() >= Duration::from_millis(800),
        "shutdown from an unrelated runtime must JOIN the busy worker \
         (returned in {:?} — the background fallback)",
        started.elapsed()
    );
    drop(unrelated);
}

#[test]
fn teardown_dup_is_cloexec_and_released_at_mark_down() {
    // Round-3 major 3: the dup must be atomically CLOEXEC (the
    // authenticated socket must not leak into children) and must be
    // RELEASED at mark_down — a retained Arc<DistConnection> holds no
    // dead descriptor after teardown.
    let (manager, atom_table) = manager();
    let sender = DistSender::new(manager.clone()).expect("sender builds");
    manager.set_runtime_handle(sender.handle());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let _client = std::net::TcpStream::connect(addr).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    let node = atom_table.intern("peer@cloexec");
    let connection = {
        let handle = sender.handle();
        let _context = handle.enter();
        manager
            .register_test_connection(node, addr, server)
            .expect("register test connection")
    };
    assert_eq!(
        connection.teardown_fd_cloexec(),
        Some(true),
        "the teardown dup is created atomically CLOEXEC"
    );
    manager.disconnect_node(node);
    assert_eq!(
        connection.teardown_fd_cloexec(),
        None,
        "mark_down releases the dup even while this Arc is retained"
    );
    sender.shutdown();
}

#[test]
fn teardown_dup_failure_refuses_the_connection() {
    // Round-3 major 3: a connection whose teardown dup cannot be created
    // must be REFUSED, never installed with a degraded closure guarantee.
    let (manager, atom_table) = manager();
    let sender = DistSender::new(manager.clone()).expect("sender builds");
    manager.set_runtime_handle(sender.handle());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let _client = std::net::TcpStream::connect(addr).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    let node = atom_table.intern("peer@dupfail");
    crate::distribution::connection::FAIL_TEARDOWN_DUP_FOR_TEST.with(|flag| flag.set(true));
    let refused = {
        let handle = sender.handle();
        let _context = handle.enter();
        manager.register_test_connection(node, addr, server)
    };
    crate::distribution::connection::FAIL_TEARDOWN_DUP_FOR_TEST.with(|flag| flag.set(false));
    assert!(refused.is_err(), "dup failure refuses the install");
    assert!(
        manager.get_connection(node).is_none(),
        "a refused connection is never tabled"
    );
    sender.shutdown();
}

#[test]
fn cross_scheduler_same_named_worker_still_joins_the_other_runtime() {
    // Round-3 major 2: every sender worker shares the "beamr-dist-send"
    // name, so identity must be per-INSTANCE (the runtime mark) — shutdown
    // of sender A from sender B's same-named worker must take the JOINED
    // path. Pinned by time: A's worker is busy in a synchronous sleep, so
    // a joined shutdown cannot return before that sleep ends, while the
    // (wrong) background path returns immediately.
    let (manager_a, _t1) = manager();
    let sender_a = DistSender::new(manager_a).expect("sender A builds");
    let (manager_b, _t2) = manager();
    let sender_b = DistSender::new(manager_b).expect("sender B builds");

    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    sender_a.handle().spawn(async move {
        let _ = busy_tx.send(());
        std::thread::sleep(Duration::from_millis(1200));
    });
    busy_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("A's busy task starts");

    let (elapsed_tx, elapsed_rx) = std::sync::mpsc::channel();
    let a_for_b = sender_a.clone();
    sender_b.handle().spawn(async move {
        let started = std::time::Instant::now();
        a_for_b.shutdown();
        let _ = elapsed_tx.send(started.elapsed());
    });
    let elapsed = elapsed_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("A's shutdown from B's worker completes");
    assert!(
        elapsed >= Duration::from_millis(800),
        "shutdown from another scheduler's same-named worker must JOIN \
         (returned in {elapsed:?} — the self-runtime background fallback)"
    );
    sender_b.shutdown();
}

#[test]
fn shutdown_from_the_senders_own_blocking_pool_does_not_deadlock() {
    // Round-3 major 2, the other half: the runtime mark is stamped on the
    // blocking pool too, so shutdown from the sender's own spawn_blocking
    // thread takes the non-deadlocking background path.
    let (manager, _atom_table) = manager();
    let sender = DistSender::new(manager).expect("sender builds");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let own = sender.clone();
    sender.handle().spawn_blocking(move || {
        own.shutdown();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("shutdown on the sender's own blocking pool must complete, not deadlock");
}

#[test]
fn shutdown_from_the_senders_own_runtime_worker_does_not_deadlock() {
    // The runtime handle is public, so a task ON the sender's sole worker
    // can trigger shutdown. A blocking join there is a self-join cycle
    // (the worker waits for the helper, the helper waits for the worker);
    // `join_runtime_drop` must detect the async context and fall back to
    // `shutdown_background`. Bounded by the harness only through the
    // channel timeout below — a regression hangs the recv, not the suite.
    let (manager, _table) = manager();
    let sender = DistSender::new(manager).unwrap_or_else(|| panic!("sender builds"));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let on_runtime = sender.clone();
    sender.handle().spawn(async move {
        on_runtime.shutdown();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("shutdown on the sender's own worker must complete, not deadlock");
    // The owner path stays idempotent afterward.
    sender.shutdown();
    assert!(sender.worker_thread_names().is_empty());
}

#[test]
fn final_clone_drop_on_the_senders_own_runtime_worker_does_not_deadlock() {
    // Same cycle through the Drop safety net: the LAST clone dropping
    // inside one of the runtime's own tasks must not block that worker on
    // its own exit.
    let (manager, _table) = manager();
    let sender = DistSender::new(manager).unwrap_or_else(|| panic!("sender builds"));
    let handle = sender.handle().clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    handle.spawn(async move {
        drop(sender);
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("final-clone drop on the sender's own worker must complete, not deadlock");
}

// ---- #64: byte-currency residency bounds on the outbound lanes ----------
//
// The tests below are the RED-FIRST artifact for lane #64 (DIST bounds
// COUNT -> BYTES). They are written against the clean base, where both
// lanes are bounded ONLY by a slot COUNT and are therefore blind to the
// bytes those slots retain.

/// Body size of one red-artifact frame: far below
/// [`MAX_DIST_FRAME_BYTES`] (so no per-frame cap is in play), yet large
/// enough that a handful of them dwarf any sane lane residency budget.
const RED_FRAME_BODY_BYTES: usize = 4 * 1024 * 1024;

/// How many red-artifact frames to offer a parked lane. Chosen so the
/// COUNT stays far below both lanes' slot caps (1024 data / 256 control)
/// while the BYTES total 256 MiB.
const RED_FRAME_COUNT: usize = 64;

/// Vacuity guard, enforced at COMPILE time: the burst those two constants
/// describe must exceed the data lane's byte budget. If it ever stopped
/// doing so, the red artifact and the byte-refusal wall would both pass
/// without a refusal ever firing — green for the wrong reason.
const _: () = assert!(
    RED_FRAME_COUNT * (8 + RED_FRAME_BODY_BYTES) > DIST_SEND_QUEUE_BYTE_BUDGET,
    "the red burst must exceed the data lane's byte budget"
);

/// A framed control frame of `body` bytes, tagged with `seq` in its first
/// byte so a reading peer can identify which frames arrived.
fn framed_of_size(seq: u8, body: usize) -> Arc<[u8]> {
    let mut control = vec![0u8; body];
    control[0] = seq;
    let control_len = u32::try_from(control.len()).expect("control fits u32");
    let mut frame = Vec::with_capacity(8 + control.len());
    frame.extend_from_slice(&control_len.to_be_bytes());
    frame.extend_from_slice(&0u32.to_be_bytes());
    frame.extend_from_slice(&control);
    Arc::from(frame.into_boxed_slice())
}

/// A peer socket that is accepted but NEVER read, plus the connection
/// registered against it. Writing more than the kernel send+recv buffers
/// can hold parks the drain until `WRITE_TIMEOUT`, which is what lets a
/// test observe what a lane RETAINS rather than what it forwards.
struct WedgedPeer {
    connection: Arc<DistConnection>,
    node: Atom,
    _held: tokio::net::TcpStream,
}

async fn wedged_peer(connections: &ConnectionManager, atom_table: &AtomTable) -> WedgedPeer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind wedged");
    let addr = listener.local_addr().expect("wedged addr");
    let node = atom_table.intern("wedged@127.0.0.1");
    let stream = std::net::TcpStream::connect(addr).expect("wedged connects");
    let peer_addr = stream.peer_addr().expect("wedged peer addr");
    let accept = tokio::spawn(async move { listener.accept().await });
    let connection = connections
        .register_test_connection(node, peer_addr, stream)
        .expect("register wedged connection");
    let (held, _) = accept
        .await
        .expect("wedged accept join")
        .expect("wedged accepted");
    WedgedPeer {
        connection,
        node,
        _held: held,
    }
}

/// One frame big enough to overflow the kernel buffers a wedged peer never
/// drains, so the drain parks on it until `WRITE_TIMEOUT`.
fn drain_parking_frame() -> Arc<[u8]> {
    framed_of_size(0x01, 16 * 1024 * 1024)
}

/// #64 D2 EVIDENCE — the encoded size of the control lane's ONLY
/// production traffic, measured at the bytes rather than read off the
/// encoder.
///
/// The control lane's producers are enumerated and closed: every frame on
/// it is minted by `scheduler::dist_control_out` through the
/// `distribution::control_link` encoders, and every one of those is
/// `{Op, FromExtPid, ToExtPid[, ReasonAtom]}` over an ALWAYS-NIL payload.
/// A control frame therefore carries no user term at all. Its only
/// variable-length components are the two node-name atoms and, for
/// EXIT/EXIT2, a reason atom drawn from `ExitReason`'s closed six-atom
/// set — so the frame has a STRUCTURAL size ceiling, set by the ETF atom
/// encoding's own `u16` length field.
///
/// This test measures both ends of that range: realistic node names, and
/// node names at the wire-permitted ceiling a hostile peer could actually
/// advertise through the handshake.
#[test]
fn control_frame_encoded_sizes_measured_at_the_bytes() {
    use crate::distribution::control_link::{
        ControlOp, encode_exit_frame, encode_link_frame, encode_unlink_frame,
    };
    use crate::process::{ExitReason, RemotePid};

    let atom_table = AtomTable::with_common_atoms();

    // The ceiling `encode_atom_name` permits: it writes ATOM_UTF8_EXT with
    // a u16 length, and the handshake refuses any advertised name longer
    // than `u16::MAX`. A hostile peer can reach exactly this.
    let ceiling_name = "n".repeat(usize::from(u16::MAX));

    let mut measurements: Vec<(String, usize)> = Vec::new();
    for (label, local_name, peer_name) in [
        ("typical", "local@127.0.0.1", "peer@127.0.0.1"),
        ("atom-ceiling", ceiling_name.as_str(), ceiling_name.as_str()),
    ] {
        let local_node = atom_table.intern(local_name);
        let to = RemotePid {
            node: atom_table.intern(peer_name),
            pid_number: u64::from(u32::MAX),
            serial: u64::from(u32::MAX),
        };
        let link = encode_link_frame(local_node, u64::from(u32::MAX), to, &atom_table)
            .expect("LINK encodes");
        let unlink = encode_unlink_frame(local_node, u64::from(u32::MAX), to, &atom_table)
            .expect("UNLINK encodes");
        // `noconnection` is the longest atom in `ExitReason`'s closed set,
        // so it is the reason worst case for both EXIT and EXIT2.
        let exit = encode_exit_frame(
            ControlOp::Exit,
            local_node,
            u64::from(u32::MAX),
            to,
            ExitReason::NoConnection,
            &atom_table,
        )
        .expect("EXIT encodes");
        let exit2 = encode_exit_frame(
            ControlOp::Exit2,
            local_node,
            u64::from(u32::MAX),
            to,
            ExitReason::NoConnection,
            &atom_table,
        )
        .expect("EXIT2 encodes");
        for (op, frame) in [
            ("LINK", &link),
            ("UNLINK", &unlink),
            ("EXIT", &exit),
            ("EXIT2", &exit2),
        ] {
            measurements.push((format!("{label}/{op}"), frame.len()));
        }
    }

    for (what, bytes) in &measurements {
        println!("#64 D2 measurement: {what} encodes to {bytes} bytes");
    }

    let worst = measurements
        .iter()
        .map(|(_, bytes)| *bytes)
        .max()
        .expect("measurements are non-empty");
    println!("#64 D2 measurement: worst-case control frame = {worst} bytes");
    let lane_worst_case = worst * DIST_CONTROL_QUEUE_CAP;
    println!(
        "#64 D2 measurement: control lane at full {DIST_CONTROL_QUEUE_CAP}-slot occupancy \
         retains at most {lane_worst_case} bytes"
    );

    // The evidence claim: a control frame is STRUCTURALLY small, so the
    // lane's slot count already implies a hard byte ceiling — and that
    // ceiling is smaller than a SINGLE maximum-size data frame.
    assert!(
        lane_worst_case < MAX_DIST_FRAME_BYTES,
        "control lane worst-case residency ({lane_worst_case} B) must be below one \
         max-size data frame ({MAX_DIST_FRAME_BYTES} B)"
    );
}

/// #64 RED (D1) — the data lane is byte-BLIND.
///
/// With the drain parked on a wedged peer, this offers the lane
/// [`RED_FRAME_COUNT`] frames of [`RED_FRAME_BODY_BYTES`] each: a COUNT of
/// 64 against a 1024-slot cap (6% full), but 256 MiB of retained bytes.
/// Every one is accepted and later delivered, because nothing on this lane
/// looks at `frame.len()`.
///
/// The property asserted is the one that SHOULD hold: a lane whose
/// residency is bounded in bytes cannot forward more than its byte budget
/// out of a single parked burst. At the clean base it does not hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_lane_bounds_retained_bytes_not_just_slot_count() {
    let (connections, atom_table) = manager();
    let wedged = wedged_peer(&connections, &atom_table).await;

    // Healthy node: read every frame and total the bytes that arrive.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live");
    let addr = listener.local_addr().expect("live addr");
    let received_bytes = Arc::new(AtomicUsize::new(0));
    let received_frames = Arc::new(AtomicUsize::new(0));
    let bytes_for_task = Arc::clone(&received_bytes);
    let frames_for_task = Arc::clone(&received_frames);
    let reader = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("live accept");
        loop {
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_err() {
                break;
            }
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_err() {
                break;
            }
            frames_for_task.fetch_add(1, Ordering::SeqCst);
            bytes_for_task.fetch_add(8 + body.len(), Ordering::SeqCst);
        }
    });
    let live_stream = std::net::TcpStream::connect(addr).expect("live connects");
    let live_node = atom_table.intern("live@127.0.0.1");
    let live_peer_addr = live_stream.peer_addr().expect("live peer addr");
    connections
        .register_test_connection(live_node, live_peer_addr, live_stream)
        .expect("register live connection");

    let sender = DistSender::new(connections.clone()).expect("sender builds");

    // Park the drain FIRST. The lane is FIFO, so this frame is taken
    // before any of the burst below and holds the drain for WRITE_TIMEOUT.
    sender.enqueue(DistOutbound::ToNode {
        node: wedged.node,
        frame: drain_parking_frame(),
    });
    // The burst: count far under the slot cap, bytes far over any budget.
    for index in 0..RED_FRAME_COUNT {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender.enqueue(DistOutbound::ToNode {
            node: live_node,
            frame: framed_of_size(seq, RED_FRAME_BODY_BYTES),
        });
    }

    // Let the parked write time out, then wait for delivery to go quiet.
    // Quiescence rather than a fixed sleep: the point of measurement is
    // "how much did this lane ultimately let through", which is only
    // settled once nothing more is arriving.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut previous = 0usize;
    let mut quiet_rounds = 0u32;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let current = received_bytes.load(Ordering::SeqCst);
        if current > 0 && current == previous {
            quiet_rounds += 1;
        } else {
            quiet_rounds = 0;
        }
        previous = current;
        if quiet_rounds >= 4 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "data lane never went quiet; {current} bytes delivered so far"
        );
    }

    let delivered_bytes = received_bytes.load(Ordering::SeqCst);
    let delivered_frames = received_frames.load(Ordering::SeqCst);
    let offered_bytes = RED_FRAME_COUNT * (8 + RED_FRAME_BODY_BYTES);
    println!(
        "#64 RED (data lane): offered {RED_FRAME_COUNT} frames / {offered_bytes} bytes \
         behind a parked drain; delivered {delivered_frames} frames / {delivered_bytes} bytes"
    );

    // The byte-bounded property. A lane that accounts in bytes cannot let a
    // single parked burst through in excess of its residency budget; a lane
    // that counts slots lets all 256 MiB through.
    let budget = 2 * MAX_DIST_FRAME_BYTES;
    assert!(
        delivered_bytes <= budget,
        "data lane must bound RETAINED BYTES, not just slots: {RED_FRAME_COUNT} frames \
         ({delivered_frames} delivered) carried {delivered_bytes} bytes through a lane \
         whose byte budget is {budget}, with the slot count at {RED_FRAME_COUNT} of \
         {DIST_SEND_QUEUE_CAP}"
    );

    sender.shutdown();
    drop(sender);
    reader.abort();
    drop(wedged);
}

/// #64 D2 WALL — the control lane's slot cap IS its byte bound.
///
/// The control-lane red (see `gate-logs/64-red/`) showed this lane is
/// byte-blind as an API surface: a synthetic caller can put a 4 MiB
/// `Arc<[u8]>` on it and nothing objects. The measurement in
/// `control_frame_encoded_sizes_measured_at_the_bytes` is what decided the
/// disposition — the frames this lane actually carries are structurally
/// ceilinged, so its slot count already bounds bytes and a second budget
/// would only add a refusal arm to a must-deliver lane.
///
/// This test is the wall on that stated absence. It floods the lane past
/// its slot cap with REAL production frames at the adversarial worst case —
/// EXIT frames whose node names sit at the ETF atom ceiling — and pins two
/// things: that the lane's only refusal arm is the slot cap, with DC-1
/// intact (overflow marks the pinned connection down, never a silent drop),
/// and that a full lane of such frames retains less than ONE maximum-size
/// data frame. If a wider frame class is ever routed here, this fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_lane_slot_cap_bounds_retained_bytes_below_one_max_data_frame() {
    use crate::distribution::control_link::{ControlOp, encode_exit_frame};
    use crate::process::{ExitReason, RemotePid};

    let (connections, atom_table) = manager();

    // The worst control frame a peer can force: node names at the ETF atom
    // ceiling, and the longest reason atom in the closed set.
    let ceiling_name = "n".repeat(usize::from(u16::MAX));
    let local_node = atom_table.intern(&ceiling_name);
    let to = RemotePid {
        node: local_node,
        pid_number: u64::from(u32::MAX),
        serial: u64::from(u32::MAX),
    };
    let worst = encode_exit_frame(
        ControlOp::Exit,
        local_node,
        u64::from(u32::MAX),
        to,
        ExitReason::NoConnection,
        &atom_table,
    )
    .expect("worst-case EXIT encodes");
    let worst_frame_bytes = worst.len();

    // The stated-absence arithmetic, over the MEASURED ceiling.
    let lane_worst_case = worst_frame_bytes * DIST_CONTROL_QUEUE_CAP;
    println!(
        "#64 D2 wall: worst-case production control frame = {worst_frame_bytes} bytes; \
         {DIST_CONTROL_QUEUE_CAP} slots retain at most {lane_worst_case} bytes"
    );
    assert!(
        lane_worst_case < MAX_DIST_FRAME_BYTES,
        "the control lane's slot cap is only a byte bound while a full lane \
         ({lane_worst_case} B) stays under one max-size data frame \
         ({MAX_DIST_FRAME_BYTES} B)"
    );

    // Behavioural half: flood past the slot cap with those real frames
    // behind a parked drain, and confirm the refusal arm is the slot cap
    // with DC-1 intact.
    let wedged = wedged_peer(&connections, &atom_table).await;
    let sender = DistSender::new(connections.clone()).expect("sender builds");

    let start = std::time::Instant::now();
    let mut accepted = 0usize;
    let mut overflowed = 0usize;
    for _ in 0..(DIST_CONTROL_QUEUE_CAP + 32) {
        // A distinct buffer each time: 256 EXIT frames bound for different
        // peers share nothing, which is the residency worst case.
        let frame: Arc<[u8]> = Arc::from(worst.clone().into_boxed_slice());
        match sender.enqueue_control(ControlOutbound {
            connection: Arc::clone(&wedged.connection),
            frame,
        }) {
            Ok(()) => accepted += 1,
            Err(ControlEnqueueError::Overflow) => {
                assert!(
                    wedged.connection.is_down(),
                    "DC-1: overflow must mark the pinned connection down before returning"
                );
                overflowed += 1;
            }
            Err(ControlEnqueueError::Closed) => {
                panic!("control lane must not close while the sender is live")
            }
        }
    }
    let elapsed = start.elapsed();

    println!(
        "#64 D2 wall: accepted {accepted}, overflowed {overflowed} \
         (slot cap {DIST_CONTROL_QUEUE_CAP})"
    );
    // Note what `accepted` is and is not. It counts CUMULATIVE admissions,
    // which legitimately exceed the slot cap: the drain keeps draining
    // while the flood runs (a wedged peer's kernel buffers absorb the first
    // frames before the write finally parks), and every frame it takes
    // frees a slot. The slot cap bounds RESIDENCY, not cumulative
    // admission, and residency is what the arithmetic above bounds in
    // bytes.
    assert!(
        overflowed > 0,
        "flooding past the slot cap behind a parked drain must overflow the lane"
    );
    assert!(
        elapsed < WRITE_TIMEOUT,
        "enqueue_control must stay non-blocking; flood took {elapsed:?}"
    );

    sender.shutdown();
    drop(sender);
    drop(wedged);
}

/// #64 D4 — every data-lane reservation is released once the drain is done
/// with it.
///
/// A leaked reservation is a slow-starve: the budget would shrink
/// permanently until the lane refused everything, with nothing actually
/// resident. Frames are driven all the way through to a reading peer, and
/// the meter must return to exactly zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_lane_releases_every_reservation_once_the_drain_completes() {
    let (connections, atom_table) = manager();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let count = 32usize;

    let reader = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        for _ in 0..count {
            let mut header = [0u8; 8];
            if stream.read_exact(&mut header).await.is_err() {
                break;
            }
            let control_len =
                u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let payload_len =
                u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
            let mut body = vec![0u8; control_len + payload_len];
            if stream.read_exact(&mut body).await.is_err() {
                break;
            }
        }
    });

    let std_stream = std::net::TcpStream::connect(addr).expect("client connects");
    let node = atom_table.intern("peer@127.0.0.1");
    let peer_addr: SocketAddr = std_stream.peer_addr().expect("peer addr");
    connections
        .register_test_connection(node, peer_addr, std_stream)
        .expect("register test connection");

    let sender = DistSender::new(connections).expect("sender builds");
    assert_eq!(
        sender.data_lane_resident_bytes(),
        0,
        "a fresh lane holds no reservation"
    );

    for index in 0..count {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender.enqueue(DistOutbound::ToNode {
            node,
            frame: framed_of_size(seq, 64 * 1024),
        });
    }

    reader.await.expect("reader task joins");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while sender.data_lane_resident_bytes() != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "every reservation must be released once the drain is done; {} bytes \
             still charged",
            sender.data_lane_resident_bytes()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    sender.shutdown();
    drop(sender);
}

/// #64 D4 — the data lane's byte refusal never blocks its caller, and never
/// lets residency past the budget.
///
/// `enqueue` is called from scheduler worker threads, so a refusal that
/// waited for room would stall a worker behind a stalled peer — the exact
/// hazard the drop semantics exist to avoid. The refusal must therefore
/// stay a prompt drop, exactly as a full slot queue already is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_lane_byte_refusal_drops_promptly_without_blocking() {
    let (connections, atom_table) = manager();
    let wedged = wedged_peer(&connections, &atom_table).await;
    let sender = DistSender::new(connections.clone()).expect("sender builds");

    // Park the drain so nothing is released while the burst runs.
    sender.enqueue(DistOutbound::ToNode {
        node: wedged.node,
        frame: drain_parking_frame(),
    });

    let start = std::time::Instant::now();
    for index in 0..RED_FRAME_COUNT {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender.enqueue(DistOutbound::ToNode {
            node: wedged.node,
            frame: framed_of_size(seq, RED_FRAME_BODY_BYTES),
        });
        assert!(
            sender.data_lane_resident_bytes() <= DIST_SEND_QUEUE_BYTE_BUDGET,
            "residency must never exceed the byte budget: {} > {}",
            sender.data_lane_resident_bytes(),
            DIST_SEND_QUEUE_BYTE_BUDGET
        );
    }
    let elapsed = start.elapsed();

    println!(
        "#64 D4: after offering {RED_FRAME_COUNT} x {RED_FRAME_BODY_BYTES} B behind a \
         parked drain, residency is {} B of a {DIST_SEND_QUEUE_BYTE_BUDGET} B budget",
        sender.data_lane_resident_bytes()
    );
    assert!(
        elapsed < WRITE_TIMEOUT,
        "enqueue must stay non-blocking under byte refusal; burst took {elapsed:?}"
    );
    sender.shutdown();
    drop(sender);
    drop(wedged);
}

/// #64 D4 — a queued byte charge must not keep the sender alive.
///
/// The module's load-bearing invariant is that the sender does not
/// transitively own itself: the drain closure captures only the
/// `ConnectionManager`. A reservation riding inside the queue would break
/// that if it referenced the sender, because the queue is owned by the
/// receiver the sender's own runtime drives — a pending frame would keep
/// the runtime alive from inside its own queue, and the blocking
/// runtime-drop join would never complete. The meter is a leaf for exactly
/// this reason; this test is the wall.
#[test]
fn queued_byte_charges_do_not_keep_the_sender_alive() {
    let (connections, atom_table) = manager();
    let node = atom_table.intern("absent@127.0.0.1");
    let sender = DistSender::new(connections).expect("sender builds");

    // Leave frames — and therefore live charges — in the queue. With no
    // connection registered the drain discards them, so enqueue faster than
    // it can keep up and drop the sender with charges still outstanding.
    for index in 0..RED_FRAME_COUNT {
        let seq = u8::try_from(index).expect("seq fits u8");
        sender.enqueue(DistOutbound::ToNode {
            node,
            frame: framed_of_size(seq, RED_FRAME_BODY_BYTES),
        });
    }

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(sender);
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("dropping a sender with charged frames still queued must complete");
}
