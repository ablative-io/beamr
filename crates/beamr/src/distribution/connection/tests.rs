use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::Instant;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::JoinHandle;

use super::frame::frame_buffer_for_header;
use super::residency::{INBOUND_RESIDENCY_ENVELOPE_BYTES, INBOUND_RESIDENCY_PER_PEER_BYTES};
use super::*;
use crate::distribution::etf::MAX_DIST_FRAME_BYTES;
use crate::distribution::handshake::{HandshakeError, HandshakeNode};
use crate::distribution::resolver::StaticResolver;

const TEST_COOKIE: &str = "test-cookie";

fn manager_with_resolver(resolver: Arc<StaticResolver>) -> ConnectionManager {
    ConnectionManager::new(
        Arc::new(AtomTable::with_common_atoms()),
        resolver,
        TEST_COOKIE,
        "local@127.0.0.1",
        1,
    )
}

/// A connection manager with the proactive net-tick enabled at test-scale
/// timings: a short interval and a deadline a few intervals long so a
/// silently-partitioned peer is detected within a bounded test window while a
/// healthy peer's keepalives still refresh liveness in time.
fn manager_with_heartbeat(
    resolver: Arc<StaticResolver>,
    interval: Duration,
    deadline: Duration,
) -> ConnectionManager {
    ConnectionManager::new(
        Arc::new(AtomTable::with_common_atoms()),
        resolver,
        TEST_COOKIE,
        "local@127.0.0.1",
        1,
    )
    .with_heartbeat(HeartbeatConfig { interval, deadline })
}

/// Accept a single inbound stream on `listener` and respond to the OTP
/// handshake advertising `name`, mirroring a real peer's accept side so the
/// outbound `connect` under test can complete its handshake.
fn spawn_responder(
    listener: TcpListener,
    name: &'static str,
    cookie: &'static str,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let local = HandshakeNode::with_default_flags(name, 7)
            .expect("responder node name should be valid");
        let _ = crate::distribution::handshake::respond_handshake_async(
            &mut stream,
            &local,
            cookie,
            99,
        )
        .await;
        // Keep the accepted stream alive so the connection is not torn down
        // while the test inspects the outbound side.
        tokio::time::sleep(Duration::from_millis(200)).await;
    })
}

/// Accept one inbound stream, complete the handshake advertising `name`, and
/// hand the accepted (still-open) stream back to the caller so a test can
/// later drop it to simulate the peer going away after a successful link.
fn spawn_responder_handoff(
    listener: TcpListener,
    name: &'static str,
) -> tokio::sync::oneshot::Receiver<TcpStream> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let local = HandshakeNode::with_default_flags(name, 7)
            .expect("responder node name should be valid");
        if crate::distribution::handshake::respond_handshake_async(
            &mut stream,
            &local,
            TEST_COOKIE,
            99,
        )
        .await
        .is_ok()
        {
            let _ = sender.send(stream);
        }
    });
    receiver
}

#[tokio::test]
async fn empty_manager_has_no_connections() {
    let manager = manager_with_resolver(Arc::new(StaticResolver::new(
        std::collections::HashMap::new(),
    )));
    let node = manager.inner.atom_table.intern("missing@127.0.0.1");

    assert_eq!(manager.connection_count(), 0);
    assert!(manager.get_connection(node).is_none());
}

#[tokio::test]
async fn outbound_connect_inserts_table_entry() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| {
            panic!("failed to bind local listener: {error}");
        });
    let addr = listener.local_addr().unwrap_or_else(|error| {
        panic!("failed to inspect local listener: {error}");
    });
    let _responder = spawn_responder(listener, "remote@127.0.0.1", TEST_COOKIE);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let connection = manager
        .connect("remote@127.0.0.1")
        .await
        .unwrap_or_else(|error| panic!("connect failed: {error}"));
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    assert!(Arc::ptr_eq(
        &connection,
        &manager
            .get_connection(node)
            .expect("connection should be present"),
    ));
}

#[tokio::test]
async fn connect_keys_table_by_remote_handshake_name_not_resolver_key() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("failed to bind local listener: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to inspect local listener: {error}"));
    // The peer advertises a DIFFERENT name than the resolver key the dialer
    // used, proving identity comes from the authenticated handshake.
    let _responder = spawn_responder(listener, "advertised@127.0.0.1", TEST_COOKIE);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "dialed@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let connection = manager
        .connect("dialed@127.0.0.1")
        .await
        .unwrap_or_else(|error| panic!("connect failed: {error}"));

    let advertised = manager.inner.atom_table.intern("advertised@127.0.0.1");
    let dialed = manager.inner.atom_table.intern("dialed@127.0.0.1");
    assert_eq!(connection.node(), advertised);
    assert!(manager.get_connection(advertised).is_some());
    assert!(
        manager.get_connection(dialed).is_none(),
        "connection must not be keyed by the resolver key"
    );
}

#[tokio::test]
async fn connect_rejects_wrong_cookie_and_records_no_entry() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("failed to bind local listener: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to inspect local listener: {error}"));
    // Responder uses a different cookie, so the handshake digest mismatches.
    let _responder = spawn_responder(listener, "remote@127.0.0.1", "other-cookie");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let result = manager.connect("remote@127.0.0.1").await;

    assert!(
        matches!(result, Err(ConnectError::Io(_))),
        "connect must fail with Io on cookie mismatch"
    );
    assert_eq!(manager.connection_count(), 0);
    let remote = manager.inner.atom_table.intern("remote@127.0.0.1");
    assert!(manager.get_connection(remote).is_none());
}

#[tokio::test]
async fn inbound_wrong_cookie_registers_no_entry() {
    // A listening manager authenticates with TEST_COOKIE. A peer that
    // initiates the handshake with a DIFFERENT cookie must be rejected by
    // the register-side accept loop (the `handle_accepted` Err -> drop arm)
    // and must NOT receive a connection-table entry.
    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::new()));
    let manager = manager_with_resolver(resolver);
    let accept = manager
        .listen("127.0.0.1:0".parse().unwrap_or_else(|error| {
            panic!("failed to parse listen address: {error}");
        }))
        .await
        .unwrap_or_else(|error| panic!("failed to start accept loop: {error}"));

    let mut client = TcpStream::connect(accept.local_addr())
        .await
        .unwrap_or_else(|error| panic!("failed to open inbound stream: {error}"));
    let client_node = HandshakeNode::with_default_flags("client@127.0.0.1", 5)
        .expect("client node name should be valid");
    // The client uses the WRONG cookie, so the digest mismatches and the
    // listening manager's responder rejects the handshake.
    let result = crate::distribution::handshake::initiate_handshake_async(
        &mut client,
        &client_node,
        "wrong-cookie",
        42,
    )
    .await;
    assert!(
        result.is_err(),
        "inbound handshake with wrong cookie must fail"
    );

    // The inbound handshake runs on a spawned task, so poll (rather than a
    // fixed sleep) to confirm the rejection never produces a table entry.
    let node = manager.inner.atom_table.intern("client@127.0.0.1");
    for _ in 0..40 {
        assert_eq!(
            manager.connection_count(),
            0,
            "wrong-cookie peer must never register a connection"
        );
        assert!(
            manager.get_connection(node).is_none(),
            "wrong-cookie peer must not appear in the connection table"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(client);
}

/// HS-1: an outbound `connect` to a peer that accepts the TCP connection but
/// never speaks the handshake must return a handshake-timeout error within the
/// configured handshake deadline, not hang. This is the bounded-return
/// contract that lets the haematite-side retry above the seam make progress.
#[tokio::test]
async fn connect_returns_timeout_when_peer_never_handshakes() {
    // A bare listener that accepts then stays silent (no responder).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("failed to bind local listener: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to inspect local listener: {error}"));
    let _silent_accept = tokio::spawn(async move {
        // Accept and hold the stream open without ever writing a handshake byte.
        if let Ok((stream, _peer)) = listener.accept().await {
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(stream);
        }
    });

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "silent@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver).with_handshake_timeout(Duration::from_secs(1));

    let started = std::time::Instant::now();
    let result =
        tokio::time::timeout(Duration::from_secs(15), manager.connect("silent@127.0.0.1")).await;

    let outcome = result
        .expect("connect must return within the handshake deadline, not hang")
        .map(|_connection| ());
    assert!(
        matches!(outcome, Err(ConnectError::Io(_))),
        "a non-speaking peer must surface as a connect error, got {outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "connect should return near the 1s handshake deadline, took {:?}",
        started.elapsed()
    );
    assert_eq!(manager.connection_count(), 0);
}

#[tokio::test]
async fn connect_node_is_idempotent_and_lists_node() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("failed to bind local listener: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to inspect local listener: {error}"));
    let _responder = spawn_responder(listener, "remote@127.0.0.1", TEST_COOKIE);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    assert!(manager.connect_node(node).await);
    assert!(manager.connect_node(node).await);
    assert_eq!(manager.connected_nodes(), vec![node]);
    assert_eq!(manager.connection_count(), 1);
}

#[tokio::test]
async fn connect_node_returns_false_for_unresolved_node() {
    let manager = manager_with_resolver(Arc::new(StaticResolver::new(
        std::collections::HashMap::new(),
    )));
    let node = manager.inner.atom_table.intern("missing@127.0.0.1");

    assert!(!manager.connect_node(node).await);
    assert!(manager.connected_nodes().is_empty());
}

#[tokio::test]
async fn inbound_peer_registers_under_its_handshake_name() {
    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::new()));
    let manager = manager_with_resolver(resolver);
    let accept = manager
        .listen("127.0.0.1:0".parse().unwrap_or_else(|error| {
            panic!("failed to parse listen address: {error}");
        }))
        .await
        .unwrap_or_else(|error| panic!("failed to start accept loop: {error}"));

    // The inbound peer initiates the handshake advertising "client@127.0.0.1".
    // The manager must register it under that authenticated name with NO
    // address-identity seam.
    let mut client = TcpStream::connect(accept.local_addr())
        .await
        .unwrap_or_else(|error| panic!("failed to open inbound stream: {error}"));
    let client_node = HandshakeNode::with_default_flags("client@127.0.0.1", 5)
        .expect("client node name should be valid");
    crate::distribution::handshake::initiate_handshake_async(
        &mut client,
        &client_node,
        TEST_COOKIE,
        42,
    )
    .await
    .expect("inbound peer handshake should succeed");

    let node = manager.inner.atom_table.intern("client@127.0.0.1");
    let mut connected = false;
    for _ in 0..40 {
        if manager.get_connection(node).is_some() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        connected,
        "inbound peer should register under its handshake name"
    );
    assert_eq!(manager.connected_nodes(), vec![node]);
    drop(client);
}

#[tokio::test]
async fn dropping_peer_removes_connection_and_notifies_once() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| {
            panic!("failed to bind local listener: {error}");
        });
    let addr = listener.local_addr().unwrap_or_else(|error| {
        panic!("failed to inspect local listener: {error}");
    });
    let remote_stream = spawn_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_hook = Arc::clone(&callback_count);
    manager.register_connection_down(move |_| {
        callback_count_for_hook.fetch_add(1, Ordering::SeqCst);
    });
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");
    let _connection = manager
        .connect("remote@127.0.0.1")
        .await
        .unwrap_or_else(|error| panic!("connect failed: {error}"));

    let remote_stream = remote_stream
        .await
        .expect("responder did not complete handshake");
    drop(remote_stream);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(manager.get_connection(node).is_none());
    assert_eq!(callback_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn manual_disconnect_removes_connection_and_notifies_once() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("failed to bind local listener: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to inspect local listener: {error}"));
    let _responder = spawn_responder(listener, "remote@127.0.0.1", TEST_COOKIE);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_hook = Arc::clone(&callback_count);
    manager.register_connection_down(move |event| {
        assert_eq!(event.reason, ConnectionDownReason::ManualDisconnect);
        callback_count_for_hook.fetch_add(1, Ordering::SeqCst);
    });
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    assert!(manager.connect_node(node).await);
    assert!(manager.disconnect_node(node));
    assert!(manager.disconnect_node(node));

    assert!(manager.get_connection(node).is_none());
    assert!(manager.connected_nodes().is_empty());
    assert_eq!(callback_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn write_error_removes_connection_and_notifies_once() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| {
            panic!("failed to bind local listener: {error}");
        });
    let addr = listener.local_addr().unwrap_or_else(|error| {
        panic!("failed to inspect local listener: {error}");
    });
    let remote_stream = spawn_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_hook = Arc::clone(&callback_count);
    manager.register_connection_down(move |_| {
        callback_count_for_hook.fetch_add(1, Ordering::SeqCst);
    });
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");
    let connection = manager
        .connect("remote@127.0.0.1")
        .await
        .unwrap_or_else(|error| panic!("connect failed: {error}"));

    let remote_stream = remote_stream
        .await
        .expect("responder did not complete handshake");
    drop(remote_stream);

    for _ in 0..8 {
        if connection.write_raw(b"probe").await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(25)).await;

    assert!(manager.get_connection(node).is_none());
    assert_eq!(callback_count.load(Ordering::SeqCst), 1);
}

/// HS-3 (tie-break direction, D1): with a competing local outbound recorded,
/// the responder emits `nok` when the local name is greater than the peer's,
/// and `ok_simultaneous` when the peer's name is greater — matching OTP's
/// literal name comparison. Drives the real accept loop so it exercises the
/// production decider (`decide_inbound_status`), forcing the `connecting`
/// marker that signals an in-flight outbound.
#[tokio::test]
async fn hs3_responder_rejects_when_local_name_is_greater() {
    // local = "zeta..." (greater); inbound peer = "alpha..." (lesser).
    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::new()));
    let manager = ConnectionManager::new(
        Arc::new(AtomTable::with_common_atoms()),
        resolver,
        TEST_COOKIE,
        "zeta@127.0.0.1",
        1,
    );
    let accept = manager
        .listen("127.0.0.1:0".parse().expect("parse listen addr"))
        .await
        .expect("start accept loop");

    // Simulate an in-flight local outbound to the inbound peer's name so the
    // decider sees the simultaneous case.
    let peer_atom = manager.inner.atom_table.intern("alpha@127.0.0.1");
    manager
        .inner
        .connecting
        .insert(peer_atom, Arc::new(AtomicBool::new(false)));

    let mut client = TcpStream::connect(accept.local_addr())
        .await
        .expect("inbound peer connects");
    let client_node =
        HandshakeNode::with_default_flags("alpha@127.0.0.1", 5).expect("client node name valid");
    let result = crate::distribution::handshake::initiate_handshake_async(
        &mut client,
        &client_node,
        TEST_COOKIE,
        42,
    )
    .await;

    // local(zeta) > peer(alpha) => responder sends `nok`, the initiator sees
    // a BadStatus("nok") abort, and no inbound link is registered.
    assert_eq!(
        result.expect_err("initiator must see the nok rejection"),
        HandshakeError::BadStatus("nok".into())
    );
    assert!(
        manager.get_connection(peer_atom).is_none(),
        "a rejected inbound must not register a connection"
    );
    drop(accept);
}

/// Spawn a one-shot responder on `listener` that always answers the status
/// step with `nok`, modelling a peer that keeps its reciprocal outbound.
fn spawn_nok_responder(listener: TcpListener) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let local =
            HandshakeNode::with_default_flags("peer@127.0.0.1", 9).expect("responder node valid");
        let _ = crate::distribution::handshake::respond_handshake_async_with(
            &mut stream,
            &local,
            TEST_COOKIE,
            3,
            |_peer_name| SimultaneousDecision::Reject,
        )
        .await;
    })
}

/// HS-3 (benign abort): an outbound `connect` that receives `nok` returns
/// `ConnectError::SimultaneousAbort` (not an Io failure), and `connect_node`
/// folds that into success so the caller does not retry-storm.
#[tokio::test]
async fn hs3_outbound_nok_is_a_benign_simultaneous_abort() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind nok responder");
    let addr = listener.local_addr().expect("inspect listener");
    let _responder = spawn_nok_responder(listener);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "peer@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);

    let outcome = manager.connect("peer@127.0.0.1").await.map(|_| ());
    assert!(
        matches!(outcome, Err(ConnectError::SimultaneousAbort)),
        "nok must surface as a benign SimultaneousAbort, got {outcome:?}"
    );
    assert_eq!(manager.connection_count(), 0);
}

/// HS-3: `connect_node` treats a `nok` simultaneous abort as success.
#[tokio::test]
async fn hs3_connect_node_treats_nok_abort_as_success() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind nok responder");
    let addr = listener.local_addr().expect("inspect listener");
    let _responder = spawn_nok_responder(listener);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "peer@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("peer@127.0.0.1");

    assert!(
        manager.connect_node(node).await,
        "connect_node must treat a nok abort as success (no retry-storm)"
    );
    // The abort registers no connection; the reciprocal inbound is the link.
    assert_eq!(manager.connection_count(), 0);
}

/// HS-2: two simultaneous installs for the same peer name must leave exactly
/// one live link — the CANONICAL-direction one — regardless of arrival order,
/// and the loser's socket must be closed (no orphan reader on a half-link).
///
/// `local@` < `peer@`, so the canonical survivor is this node's INBOUND
/// (the higher-named peer's outbound). The non-canonical OUTBOUND half is
/// installed FIRST here; the canonical inbound must then displace it — proving
/// the survivor is chosen by name comparison, not by who registered first.
/// Both nodes apply the same rule, so the same single TCP socket survives on
/// both ends and a pair can never lose both links.
#[tokio::test]
async fn hs2_two_simultaneous_installs_keep_exactly_one_no_orphan_reader() {
    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::new()));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("peer@127.0.0.1");

    // Two independent connected socket pairs standing in for the inbound and
    // outbound halves of a simultaneous connect. The client ends let us
    // observe whether each server end stays open or is closed.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind helper listener");
    let addr = listener.local_addr().expect("inspect helper listener");

    // First (non-canonical) install: the OUTBOUND half.
    let mut client_outbound = TcpStream::connect(addr)
        .await
        .expect("client_outbound connects");
    let (server_outbound, _) = listener.accept().await.expect("accept server_outbound");
    // Second (canonical) install: the INBOUND half, which must win.
    let mut client_inbound = TcpStream::connect(addr)
        .await
        .expect("client_inbound connects");
    let (server_inbound, _) = listener.accept().await.expect("accept server_inbound");

    // `None` admission on both: these streams are handed to the installer
    // directly, never through `accept_loop`, so neither carries a
    // reservation.
    let displaced = manager
        .register_connection(
            node,
            addr,
            server_outbound,
            LinkDirection::Outbound,
            0,
            None,
        )
        .expect("outbound installs");
    let winner = manager
        .register_connection(node, addr, server_inbound, LinkDirection::Inbound, 0, None)
        .expect("inbound installs");

    // Exactly one table entry, and it is the canonical (inbound) winner, not
    // the first-installed outbound.
    assert_eq!(manager.connection_count(), 1);
    assert!(
        !Arc::ptr_eq(&winner, &displaced),
        "the canonical inbound must displace the non-canonical outbound"
    );
    assert!(Arc::ptr_eq(
        &winner,
        &manager
            .get_connection(node)
            .expect("survivor must be in the table"),
    ));

    // The winner's socket stays open: a write reaches its peer.
    winner
        .write_raw(&[0_u8; 8])
        .await
        .expect("winner link must remain writable");
    let mut header = [0_u8; 8];
    client_inbound
        .read_exact(&mut header)
        .await
        .expect("winner's peer must receive the keepalive frame");

    // The displaced link's read half was torn down (no orphan reader). Drop
    // the last `DistConnection` Arc so its write half also closes, then the
    // peer observes EOF rather than a live, orphaned half-link.
    drop(displaced);
    let mut byte = [0_u8; 1];
    let eof = tokio::time::timeout(Duration::from_secs(5), client_outbound.read(&mut byte))
        .await
        .expect("displaced socket should close promptly, not hang")
        .expect("reading the closed displaced socket should not error");
    assert_eq!(eof, 0, "the displaced link's socket must be closed (EOF)");
}

/// Accept inbound streams on `listener` in a loop, completing the OTP
/// handshake advertising `name` for each, and hand every accepted (still-open)
/// stream back over the returned channel. Unlike [`spawn_responder_handoff`]
/// (single accept), this models a real peer that stays up across a re-dial: the
/// test can drop the first handed-back stream to simulate the link dropping,
/// then receive the second stream produced by the reconnect's fresh inbound.
fn spawn_multi_responder_handoff(
    listener: TcpListener,
    name: &'static str,
) -> tokio::sync::mpsc::UnboundedReceiver<TcpStream> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                return;
            };
            let local = HandshakeNode::with_default_flags(name, 7)
                .expect("responder node name should be valid");
            if crate::distribution::handshake::respond_handshake_async(
                &mut stream,
                &local,
                TEST_COOKIE,
                99,
            )
            .await
            .is_ok()
            {
                if sender.send(stream).is_err() {
                    return;
                }
            } else {
                return;
            }
        }
    });
    receiver
}

/// HS-4: after a distribution link drops (peer closed), a fresh `connect`
/// re-establishes the link, the stale table entry is replaced by a NEW
/// connection (not the dead one), and the new link is writable end-to-end. This
/// is the core reconnection-hardening contract: a dropped link can be re-dialed
/// deterministically and the result is a whole, usable link.
#[tokio::test]
async fn hs4_redial_after_drop_reestablishes_writable_link() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind responder listener");
    let addr = listener.local_addr().expect("inspect listener");
    let mut streams = spawn_multi_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    // First link.
    let first = manager
        .connect("remote@127.0.0.1")
        .await
        .expect("first connect should succeed");
    let first_remote = streams.recv().await.expect("first inbound handed back");

    // Drop the peer's side; our read loop observes EOF and reaps the entry.
    drop(first_remote);
    let deadline = Instant::now() + Duration::from_secs(5);
    while manager.get_connection(node).is_some() {
        assert!(Instant::now() < deadline, "dropped link was never reaped");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Re-dial: a clean re-establish, NOT a return of the dead connection.
    let second = manager
        .connect("remote@127.0.0.1")
        .await
        .expect("re-dial after drop should succeed");
    let mut second_remote = streams.recv().await.expect("second inbound handed back");

    assert!(
        !Arc::ptr_eq(&first, &second),
        "re-dial must install a NEW connection, not resurrect the dead one"
    );
    assert!(first.is_down(), "the first link must be marked down");
    assert!(!second.is_down(), "the re-dialed link must be live");
    assert_eq!(manager.connection_count(), 1, "exactly one live link");
    assert!(Arc::ptr_eq(
        &second,
        &manager
            .get_connection(node)
            .expect("re-dialed link must be in the table"),
    ));

    // The new link is writable end-to-end: an 8-byte zero header reaches the
    // peer's (re-dialed) inbound socket.
    second
        .write_raw(&[0_u8; 8])
        .await
        .expect("re-dialed link must be writable");
    let mut header = [0_u8; 8];
    tokio::time::timeout(
        Duration::from_secs(5),
        second_remote.read_exact(&mut header),
    )
    .await
    .expect("re-dialed peer must receive the frame, not hang")
    .expect("re-dialed peer read must not error");

    // The `connecting` guard cleared on every dial: no stuck in-flight marker.
    assert_eq!(
        manager.inner.connecting.len(),
        0,
        "re-dial must not leak the connecting guard"
    );
}

/// HS-4: `connect_node` must not report a DOWN-but-not-yet-reaped link as
/// connected. If it did, a caller's reconnect would be told the peer is up and
/// would never re-dial. With a stale down entry still in the table,
/// `connect_node` must run a fresh handshake and replace it.
#[tokio::test]
async fn hs4_connect_node_redials_a_down_but_unreaped_entry() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind responder listener");
    let addr = listener.local_addr().expect("inspect listener");
    let mut streams = spawn_multi_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    let first = manager
        .connect("remote@127.0.0.1")
        .await
        .expect("first connect should succeed");
    let _first_remote = streams.recv().await.expect("first inbound handed back");

    // Flip the link to down WITHOUT removing it from the table: this is the
    // narrow race window between `mark_down` and `connection_down`'s reap. We
    // reproduce it deterministically by holding a down entry in place.
    first.down.store(true, Ordering::Release);
    assert!(
        manager.get_connection(node).is_some(),
        "the stale down entry is still in the table"
    );

    // connect_node must NOT short-circuit on the down entry; it must re-dial.
    assert!(
        manager.connect_node(node).await,
        "connect_node must re-dial a down-but-unreaped entry"
    );
    let _second_remote = streams.recv().await.expect("re-dial inbound handed back");

    assert_eq!(manager.connection_count(), 1, "exactly one live link");
    let live = manager
        .get_connection(node)
        .expect("re-dialed link present");
    assert!(!live.is_down(), "the table now holds a live re-dialed link");
    assert!(
        !Arc::ptr_eq(&first, &live),
        "the dead entry must have been replaced, not reused"
    );
    assert_eq!(manager.inner.connecting.len(), 0);
}

/// HS-4: every `connect` exit path clears the `connecting` guard, so a series
/// of dials (success, hard failure, and benign `nok` abort) never leaves a
/// stuck in-flight marker that would corrupt the simultaneous-connect decider
/// or block a future re-dial.
#[tokio::test]
async fn hs4_connecting_guard_clears_on_every_exit_path() {
    // Success path.
    let ok_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ok");
    let ok_addr = ok_listener.local_addr().expect("ok addr");
    let _ok = spawn_responder(ok_listener, "ok@127.0.0.1", TEST_COOKIE);

    // nok (benign abort) path.
    let nok_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind nok");
    let nok_addr = nok_listener.local_addr().expect("nok addr");
    let _nok = spawn_nok_responder(nok_listener);

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([
        ("ok@127.0.0.1".to_string(), ok_addr),
        ("peer@127.0.0.1".to_string(), nok_addr),
        // refused@ has no listener bound -> connection refused / io error path.
    ])));
    let manager = manager_with_resolver(resolver);

    // Success.
    manager
        .connect("ok@127.0.0.1")
        .await
        .expect("ok connect should succeed");
    assert_eq!(
        manager.inner.connecting.len(),
        0,
        "success path must clear the connecting guard"
    );

    // Benign nok abort.
    assert!(matches!(
        manager.connect("peer@127.0.0.1").await,
        Err(ConnectError::SimultaneousAbort)
    ));
    assert_eq!(
        manager.inner.connecting.len(),
        0,
        "nok abort path must clear the connecting guard"
    );

    // Hard failure: unresolved name never reaches the guard, but a resolvable
    // name with no listener exercises the TCP-connect failure exit with the
    // guard already armed. Bind then immediately drop a listener to free a
    // port that now refuses.
    let dead = TcpListener::bind("127.0.0.1:0").await.expect("bind dead");
    let dead_addr = dead.local_addr().expect("dead addr");
    drop(dead);
    let resolver2 = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "dead@127.0.0.1".to_string(),
        dead_addr,
    )])));
    let manager2 = manager_with_resolver(resolver2);
    let failed = manager2.connect("dead@127.0.0.1").await;
    assert!(failed.is_err(), "connect to a refused port must fail");
    assert_eq!(
        manager2.inner.connecting.len(),
        0,
        "TCP-failure path must clear the connecting guard"
    );
}

type Resolver = Arc<dyn NodeResolver + Send + Sync>;

/// HS-0 (deterministic root-cause oracle): an inbound peer completes the TCP
/// connect then sends nothing, so the accept-side responder's first read sits
/// on an untimed `read_exact`. Pre-HS-1 that responder task never resolves and
/// the silent peer's socket stays open forever — the canonical handshake hang
/// that, multiplied across a `>=3`-node mesh of blocking dials, wedges a
/// cluster. After HS-1 the responder hits the whole-handshake deadline, the
/// server drops the stream, and the silent peer observes EOF.
///
/// The oracle drives the REAL `ConnectionManager` accept loop (so it exercises
/// the production timeout path, not a test-local wrapper) with a short
/// handshake deadline, then reads the silent peer's socket under an inner
/// bound. Pre-HS-1 the read never returns and the bound fires → failure,
/// demonstrating the hang. Post-HS-1 the read returns EOF promptly → pass. A
/// whole-test wall-clock watchdog guards against any hang escaping the bound.
#[test]
fn hs0_silent_peer_handshake_terminates_and_does_not_hang() {
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_silent_peer_scenario();
        let _ = done_tx.send(());
    });
    match done_rx.recv_timeout(Duration::from_secs(45)) {
        Ok(()) => worker.join().expect("HS-0 worker thread should not panic"),
        Err(_) => panic!(
            "HS-0 DEADLOCK: a silent peer's inbound handshake never terminated \
             (untimed read parked the responder forever)"
        ),
    }
}

fn run_silent_peer_scenario() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build handshake runtime");
    runtime.block_on(async {
        let resolver: Resolver = Arc::new(StaticResolver::new(HashMap::new()));
        // Short handshake deadline so the post-fix path resolves quickly; the
        // pre-fix path has no deadline at all and hangs regardless.
        let manager = ConnectionManager::new(
            Arc::new(AtomTable::with_common_atoms()),
            resolver,
            TEST_COOKIE,
            "server@127.0.0.1",
            1,
        )
        .with_handshake_timeout(Duration::from_secs(2));
        let accept = manager
            .listen("127.0.0.1:0".parse().expect("parse listen addr"))
            .await
            .expect("start accept loop");

        // Silent peer: connect, then never send a single byte. The accept loop
        // spawns a responder that blocks on the first handshake read.
        let mut silent = TcpStream::connect(accept.local_addr())
            .await
            .expect("silent peer connects");

        // Pre-HS-1 the responder never times out, so the server never closes
        // the socket and this read blocks forever (caught by the inner bound).
        // Post-HS-1 the responder hits the deadline, the server drops the
        // stream, and this read returns EOF (Ok(0)).
        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(15), silent.read(&mut byte)).await;

        let read = read.expect(
            "silent peer's socket was never closed: the inbound responder \
             parked on an untimed handshake read (HS-1 not in effect)",
        );
        assert_eq!(
            read.expect("reading the closed socket should not error"),
            0,
            "expected EOF after the responder timed out and dropped the stream"
        );

        // No connection should have been registered for the silent peer.
        assert_eq!(manager.connection_count(), 0);
        drop(accept);
    });
}

/// HS-0 (convergence): a 3-node full mesh, every node dialing its two peers
/// simultaneously (barrier-released) from synchronous threads via
/// `runtime.block_on` — the haematite seam. Each node's accept/responder
/// tasks share its single worker. After HS-3 exactly one link survives per
/// pair (no last-writer-wins clobber) and that link is usable in both
/// directions. Pre-fix this can deadlock or leave mismatched half-links;
/// run under a hard watchdog so a hang fails the test.
#[test]
fn hs0_three_node_simultaneous_dial_mesh_forms_without_deadlock() {
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_three_node_mesh();
        let _ = done_tx.send(());
    });
    match done_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(()) => worker.join().expect("mesh worker thread should not panic"),
        Err(_) => panic!(
            "HS-0 DEADLOCK: 3-node simultaneous-dial mesh did not converge \
             within the watchdog window (connect never returned)"
        ),
    }
}

fn run_three_node_mesh() {
    let names = ["alpha@127.0.0.1", "bravo@127.0.0.1", "charlie@127.0.0.1"];
    // Bind every listener first so the shared resolver maps all names.
    let mut prepared = Vec::new();
    let mut address_map = HashMap::new();
    for name in names {
        let runtime = Arc::new(
            Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build single-worker node runtime"),
        );
        let listener = runtime
            .block_on(TcpListener::bind("127.0.0.1:0"))
            .expect("bind node listener");
        address_map.insert(name.to_string(), listener.local_addr().expect("addr"));
        prepared.push((name, runtime, listener));
    }
    let resolver: Resolver = Arc::new(StaticResolver::new(address_map));

    let mut nodes = Vec::new();
    for (name, runtime, listener) in prepared {
        let manager = ConnectionManager::new(
            Arc::new(AtomTable::with_common_atoms()),
            Arc::clone(&resolver),
            TEST_COOKIE,
            name,
            1,
        );
        manager.set_runtime_handle(runtime.handle().clone());
        // Count control frames this node's read loops actually deliver. A
        // delivered frame proves the link is whole: the socket this node holds
        // for the peer is the same one the peer reads from. The pre-HS-2/3
        // last-writer-wins clobber can orphan one socket's reader, so a frame
        // written to the surviving write half is never observed here.
        let received = Arc::new(AtomicUsize::new(0));
        let received_for_handler = Arc::clone(&received);
        manager.register_control_frame_handler(move |_control, _payload| {
            received_for_handler.fetch_add(1, Ordering::SeqCst);
        });
        let accept = runtime.block_on(async { manager.listen_with(listener) });
        nodes.push((name, manager, runtime, accept, received));
    }

    // 3 nodes x 2 peers = 6 dialing threads, released together.
    let barrier = Arc::new(Barrier::new(6));
    let mut dialers = Vec::new();
    for (name, manager, runtime, _accept, _received) in &nodes {
        for peer in names {
            if peer == *name {
                continue;
            }
            let manager = manager.clone();
            let runtime = Arc::clone(runtime);
            let barrier = Arc::clone(&barrier);
            let peer_name = peer.to_string();
            dialers.push(thread::spawn(move || {
                barrier.wait();
                let _ = runtime.block_on(manager.connect(&peer_name));
            }));
        }
    }
    for dialer in dialers {
        dialer
            .join()
            .expect("dialer thread should not panic (connect must return)");
    }

    // Exactly one link per pair on every node. Poll: the losing inbound may
    // still be tearing down when the winning `connect` returns.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if nodes
            .iter()
            .all(|(_, manager, _, _, _)| manager.connection_count() == 2)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "mesh did not converge to one link per pair: counts = {:?}",
            nodes
                .iter()
                .map(|(_, manager, _, _, _)| manager.connection_count())
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(25));
    }

    // Every directed edge must carry a frame end-to-end. Each node writes one
    // 8-byte zero header (a zero-length control+payload frame) to each peer
    // link; each node must then OBSERVE the two frames its peers sent it. A
    // clobbered half-link silently drops the frame, so the receiver's count
    // stays below 2 and this fails — the deterministic pre-fix symptom.
    for (name, manager, runtime, _accept, _received) in &nodes {
        for peer in names {
            if peer == *name {
                continue;
            }
            let peer_atom = manager.inner.atom_table.intern(peer);
            let connection = manager
                .get_connection(peer_atom)
                .unwrap_or_else(|| panic!("{name} has no link to {peer}"));
            runtime
                .block_on(connection.write_raw(&[0_u8; 8]))
                .unwrap_or_else(|error| {
                    panic!("{name} -> {peer} surviving link not writable: {error}")
                });
        }
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if nodes
            .iter()
            .all(|(_, _, _, _, received)| received.load(Ordering::SeqCst) >= 2)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "mesh links are not whole bidirectionally: per-node received \
             frame counts = {:?} (expected >= 2 each)",
            nodes
                .iter()
                .map(|(_, _, _, _, received)| received.load(Ordering::SeqCst))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

/// Part B CONTRACT (the bug): WITHOUT the proactive net-tick, a link to a
/// silently-partitioned (black-holed) peer — one whose socket stays open but
/// sends nothing and no TCP FIN/RST arrives — is NEVER marked down. The read
/// loop blocks in `read_exact` forever, so `connected_nodes()` keeps listing
/// a dead peer and the connection-down hook (pg-purge / monitor-DOWN) never
/// fires. This pins the gap the net-tick closes.
#[tokio::test]
async fn without_net_tick_black_holed_peer_is_never_marked_down() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind responder listener");
    let addr = listener.local_addr().expect("inspect listener");
    let handoff = spawn_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    let manager = manager_with_resolver(resolver);
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    let connection = manager
        .connect("remote@127.0.0.1")
        .await
        .expect("connect should succeed");
    // Hold the peer's accepted stream open WITHOUT ever reading or writing it:
    // a silent partition (no FIN/RST). The peer task has already returned.
    let _black_holed_peer = handoff.await.expect("peer hands back its open stream");

    // Over a window many times longer than any plausible net-tick deadline,
    // the link stays up: no heartbeat means no proactive liveness check.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !connection.is_down(),
        "without a net-tick, a black-holed link is never detected as down"
    );
    assert!(
        manager.connected_nodes().contains(&node),
        "without a net-tick, connected_nodes keeps listing the dead peer"
    );
}

/// Part B FIX: WITH the proactive net-tick enabled, a link to a
/// silently-partitioned peer is marked down within a bounded deadline via the
/// EXISTING `mark_down` path — so `connected_nodes()` drops it and the
/// connection-down hook fires, exactly as a real read EOF would. The black-
/// holed peer never sends a keepalive, so the inbound-liveness deadline lapses.
#[tokio::test]
async fn net_tick_marks_black_holed_peer_down_within_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind responder listener");
    let addr = listener.local_addr().expect("inspect listener");
    let handoff = spawn_responder_handoff(listener, "remote@127.0.0.1");

    let resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        addr,
    )])));
    // Test-scale net-tick: 50ms interval, 200ms deadline.
    let manager = manager_with_heartbeat(
        resolver,
        Duration::from_millis(50),
        Duration::from_millis(200),
    );
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");

    // Observe the connection-down hook firing for the black-holed peer.
    let down_fired = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&down_fired);
    manager.register_connection_down(move |event| {
        if event.reason == ConnectionDownReason::HeartbeatTimeout {
            observed.store(true, Ordering::SeqCst);
        }
    });

    let connection = manager
        .connect("remote@127.0.0.1")
        .await
        .expect("connect should succeed");
    // Hold the peer's stream open but silent — never read, never write.
    let _black_holed_peer = handoff.await.expect("peer hands back its open stream");

    // Within a bounded window (a few deadlines) the net-tick must mark the
    // link down and reap it from the table.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !connection.is_down() {
        assert!(
            Instant::now() < deadline,
            "net-tick must mark a black-holed link down within the deadline"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The down path reaped the table entry and fired the hook.
    while manager.connected_nodes().contains(&node) {
        assert!(
            Instant::now() < deadline,
            "the downed link must be removed from connected_nodes"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        down_fired.load(Ordering::SeqCst),
        "the connection-down hook must fire with HeartbeatTimeout"
    );
}

/// No false nodedowns: WITH the net-tick enabled on BOTH peers, a healthy but
/// otherwise idle link (no application traffic) stays up indefinitely, because
/// each side's periodic keepalive refreshes the other's inbound-liveness clock
/// well within the deadline. This guards against the net-tick spuriously
/// downing quiet-but-live links.
#[tokio::test]
async fn net_tick_keeps_healthy_idle_link_up() {
    // Build a REAL peer manager (also heartbeat-enabled) so both sides emit
    // keepalives, modelling a healthy bidirectional idle link. The remote
    // binds its own listener on an ephemeral port via `listen`.
    let remote = ConnectionManager::new(
        Arc::new(AtomTable::with_common_atoms()),
        Arc::new(StaticResolver::new(std::collections::HashMap::new())),
        TEST_COOKIE,
        "remote@127.0.0.1",
        7,
    )
    .with_heartbeat(HeartbeatConfig {
        interval: Duration::from_millis(50),
        deadline: Duration::from_millis(200),
    });
    let accept = remote
        .listen("127.0.0.1:0".parse().expect("listen address parses"))
        .await
        .expect("remote node listens");
    let remote_addr = accept.local_addr();

    let local_resolver = Arc::new(StaticResolver::new(std::collections::HashMap::from([(
        "remote@127.0.0.1".to_string(),
        remote_addr,
    )])));
    let local = manager_with_heartbeat(
        local_resolver,
        Duration::from_millis(50),
        Duration::from_millis(200),
    );

    let node = local.inner.atom_table.intern("remote@127.0.0.1");
    let connection = local
        .connect("remote@127.0.0.1")
        .await
        .expect("connect should succeed");

    // Over a window many deadlines long, both keepalives keep the link live.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !connection.is_down(),
        "a healthy idle link with bidirectional keepalives must stay up"
    );
    assert!(
        local.connected_nodes().contains(&node),
        "a healthy idle link must remain in connected_nodes"
    );

    accept.shutdown();
}

/// A blocking-socket pair: `(server, client, server_addr)`. The client end
/// is the caller's handle on "the peer" and must be held for as long as the
/// link is meant to look live.
fn blocking_socket_pair() -> (std::net::TcpStream, std::net::TcpStream, SocketAddr) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind pair listener");
    let addr = listener.local_addr().expect("inspect pair listener");
    let client = std::net::TcpStream::connect(addr).expect("connect pair client");
    let (server, _) = listener.accept().expect("accept pair server");
    (server, client, addr)
}

/// Build the 8-byte data-frame header for `(control_len, payload_len)`.
fn frame_header(control_len: u32, payload_len: u32) -> [u8; 8] {
    let mut header = [0_u8; 8];
    header[..4].copy_from_slice(&control_len.to_be_bytes());
    header[4..].copy_from_slice(&payload_len.to_be_bytes());
    header
}

/// The LIVE framing path — not just the decoder — must refuse an over-cap
/// length header. Pre-cap the read loop allocated the peer-named byte count
/// and then parked in `read_exact` waiting for a body the peer need never
/// send, so the link stayed UP holding the buffer.
///
/// This test names NO symbol introduced by the fix. The over-cap total is
/// the literal `64 * 1024 * 1024 + 1`, split across the two header fields;
/// the literal deliberately mirrors the cap so that the test PREDATES it and
/// compiles unchanged at the pre-fix tree. It asserts TERMINATION only —
/// never the typed reason — so it survives any renaming of the refusal.
#[tokio::test]
async fn over_cap_frame_header_retires_the_live_link() {
    use std::io::Write as _;

    let manager = manager_with_resolver(Arc::new(StaticResolver::new(HashMap::new())));
    let node = manager.inner.atom_table.intern("remote@127.0.0.1");
    let (server, mut peer, addr) = blocking_socket_pair();
    let connection = manager
        .register_test_connection(node, addr, server)
        .expect("register test connection");

    // One byte over the cap, and a body this test never sends.
    let header = frame_header(64 * 1024 * 1024, 1);
    peer.write_all(&header).expect("write over-cap header");
    peer.flush().expect("flush over-cap header");

    let deadline = Instant::now() + Duration::from_secs(5);
    while !connection.is_down() {
        assert!(
            Instant::now() < deadline,
            "an over-cap frame length must retire the link, not park the read \
             loop holding a peer-named allocation"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Held to the very end: the link must go down on the cap, never on an
    // EOF this test handed it.
    drop(peer);
}

/// The refusal is TYPED and carries both counts — the offending total and
/// the cap in force — so an operator reading the failure learns the size
/// that was rejected and the bound that rejected it, not merely that a read
/// failed.
#[test]
fn over_cap_frame_header_is_a_typed_refusal_carrying_both_counts() {
    let cap = u32::try_from(MAX_DIST_FRAME_BYTES).expect("the cap fits a u32 header field");

    assert_eq!(
        frame_buffer_for_header(frame_header(cap, 1)),
        Err(FrameError::FrameTooLarge {
            frame_bytes: MAX_DIST_FRAME_BYTES + 1,
            max_frame_bytes: MAX_DIST_FRAME_BYTES,
        }),
        "one byte over the cap is refused, naming both counts"
    );
}

/// The boundary is `>`, not `>=`: a frame of exactly the cap is legal and is
/// sized exactly, with the control/payload split preserved.
#[test]
fn frame_header_at_exactly_the_cap_is_accepted_and_sized_exactly() {
    let cap = u32::try_from(MAX_DIST_FRAME_BYTES).expect("the cap fits a u32 header field");
    let header = frame_header(1, cap - 1);

    let (control_len, frame) =
        frame_buffer_for_header(header).expect("a frame of exactly the cap is accepted");

    assert_eq!(control_len, 1, "the control/payload split is preserved");
    assert_eq!(
        frame.len(),
        MAX_DIST_FRAME_BYTES,
        "the buffer is sized to the declared total, exactly"
    );
}

// ---- #64 D5: the accept-side residency bound ---------------------------

/// The peer ceiling is DERIVED from two byte quantities, and it is 64.
///
/// This is the ONLY place the number 64 appears in connection.rs, and it
/// appears as a derivation RESULT being checked — never as a configured
/// peer count. If either byte quantity moves, this test says so.
#[test]
fn the_accept_envelope_derives_the_sixty_four_peer_design_target() {
    let derived = INBOUND_RESIDENCY_ENVELOPE_BYTES / INBOUND_RESIDENCY_PER_PEER_BYTES;
    assert_eq!(
        derived, 64,
        "4 GiB of spike allowance divided by one 64 MiB framed buffer is the \
         64-peer design target the MAX_DIST_FRAME_BYTES derivation names"
    );
    assert_eq!(
        INBOUND_RESIDENCY_PER_PEER_BYTES, MAX_DIST_FRAME_BYTES as u64,
        "one peer's worst-case residency is exactly one framed buffer, so the \
         per-peer currency must stay tied to the frame cap"
    );
}

/// #64 D5 — `accept_loop` bounds the population it admits in residency
/// BYTES, and declines beyond the envelope.
///
/// One more stream than the envelope can hold is opened, and none of them
/// sends handshake bytes: every accepted stream parks in the responder,
/// holding its reservation for the whole handshake deadline. That is what
/// makes the overflow observable without needing real peers.
///
/// Also the leak-free wall: once the parked responders time out and drop
/// their streams, every reservation must come back. A reservation that did
/// not return would be a slow-starve — the node would refuse peers forever
/// while holding nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accept_loop_declines_inbound_peers_beyond_the_residency_envelope() {
    let (manager, handle) = ConnectionManager::start(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        Arc::new(StaticResolver::new(HashMap::new())),
        TEST_COOKIE,
        "local@127.0.0.1",
        1,
    )
    .await
    .expect("listener starts");
    let addr = handle.local_addr();

    assert_eq!(
        manager.inbound_residency_bytes(),
        0,
        "a fresh manager holds no inbound reservation"
    );
    assert_eq!(manager.inbound_accepts_refused(), 0);

    // Computed the way the accept path computes it — no literal count.
    let ceiling = INBOUND_RESIDENCY_ENVELOPE_BYTES / INBOUND_RESIDENCY_PER_PEER_BYTES;
    let over_subscribe =
        usize::try_from(ceiling + 1).expect("the derived peer ceiling fits a usize");

    // Hold every stream open and send nothing on any of them.
    let mut streams = Vec::with_capacity(over_subscribe);
    for _ in 0..over_subscribe {
        streams.push(
            TcpStream::connect(addr)
                .await
                .expect("client stream connects"),
        );
    }

    // Wait for the accept loop to work through the backlog and refuse the
    // stream that does not fit. Bounded well inside the handshake deadline
    // that holds the reservations open.
    let deadline = Instant::now() + Duration::from_secs(4);
    while manager.inbound_accepts_refused() == 0 {
        assert!(
            Instant::now() < deadline,
            "accepting {over_subscribe} streams against a {ceiling}-peer envelope must \
             refuse at least one; residency was {} bytes",
            manager.inbound_residency_bytes()
        );
        assert!(
            manager.inbound_residency_bytes() <= INBOUND_RESIDENCY_ENVELOPE_BYTES,
            "inbound residency must never exceed the envelope"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        manager.inbound_residency_bytes() <= INBOUND_RESIDENCY_ENVELOPE_BYTES,
        "inbound residency must never exceed the envelope, got {} against {}",
        manager.inbound_residency_bytes(),
        INBOUND_RESIDENCY_ENVELOPE_BYTES
    );

    // Leak-free: the parked responders time out (HS-1), drop their streams,
    // and every reservation returns.
    drop(streams);
    let release_deadline = Instant::now() + Duration::from_secs(30);
    while manager.inbound_residency_bytes() != 0 {
        assert!(
            Instant::now() < release_deadline,
            "every accept reservation must be released once its stream's lifecycle \
             ends; {} bytes still held",
            manager.inbound_residency_bytes()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    handle.shutdown();
    manager.disconnect_all();
}
