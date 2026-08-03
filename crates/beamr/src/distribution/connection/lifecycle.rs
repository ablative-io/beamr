use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::io::AsyncReadExt;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::distribution::handshake::respond_handshake_async_with;

use super::frame::{KEEPALIVE_FRAME, frame_buffer_for_header};
use super::residency::InboundAdmissionPermit;
use super::{ConnectionDownReason, ConnectionManager, DistConnection, FrameError, LinkDirection};

impl ConnectionManager {
    pub(super) fn spawn_read_lifecycle(
        &self,
        connection: Arc<DistConnection>,
        mut read_half: OwnedReadHalf,
        admission: Option<InboundAdmissionPermit>,
    ) {
        // A fresh link is live now; seed its inbound clock and start its net-tick.
        connection.note_inbound_activity();
        self.spawn_heartbeat(Arc::clone(&connection));
        let manager = Arc::clone(&self.inner);
        let shutdown = Arc::clone(&connection.shutdown);
        self.inner.spawn_lifecycle(async move {
            // D5: an INBOUND link's accept-side residency reservation lives
            // here, moved into the task that runs for exactly this link's
            // lifetime — the loop below exits on peer EOF, on a read error, or
            // when `mark_down` fires the shutdown `Notify`. Whichever exit is
            // taken, and equally if the whole task is dropped at runtime
            // teardown, the permit drops with the task and returns this peer's
            // share of the envelope. `None` for an outbound dial, which the
            // accept bound does not charge.
            let _admission = admission;
            // A single long-lived `Notified` future, re-polled via `&mut` each
            // iteration so `notify_waiters` (which wakes only already-registered
            // waiters) is never missed mid-loop. `enable()` registers the waiter
            // NOW rather than on first poll inside the select below — otherwise a
            // `notify_waiters` racing the first iteration (after the `is_down`
            // check, before the first poll) would be lost and the read loop would
            // park until peer EOF instead of dropping a displaced link promptly.
            let notified = shutdown.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            loop {
                let mut header = [0_u8; 8];
                // Race the header read against a shutdown so a retired link (e.g.
                // displaced by a simultaneous-connect canonical winner) drops its
                // read half promptly instead of parking until the peer closes.
                if connection.is_down() {
                    break;
                }
                let read_header = tokio::select! {
                    biased;
                    () = &mut notified => break,
                    result = read_half.read_exact(&mut header) => result,
                };
                match read_header {
                    Ok(_) => {
                        // Any inbound bytes (data frame OR keepalive) refresh the
                        // net-tick liveness clock for this link.
                        connection.note_inbound_activity();
                        // The header's two lengths are peer-controlled: size,
                        // cap and allocate in one place, before a single byte
                        // of the declared total is committed.
                        let (control_len, mut frame) = match frame_buffer_for_header(header) {
                            Ok(sized) => sized,
                            // Every framing refusal is terminal — no frame
                            // boundary was established, so the loop has nothing
                            // to resynchronise on. Listed variant by variant so
                            // a future one cannot be silently swallowed here.
                            Err(
                                FrameError::LengthOverflow
                                | FrameError::FrameTooLarge { .. }
                                | FrameError::AllocationFailed { .. },
                            ) => {
                                connection.mark_down(ConnectionDownReason::ReadError);
                                break;
                            }
                        };
                        if read_half.read_exact(&mut frame).await.is_err() {
                            connection.mark_down(ConnectionDownReason::ReadError);
                            break;
                        }
                        let handler = manager
                            .control_frame_handler
                            .read()
                            .unwrap_or_else(|error| error.into_inner())
                            .clone();
                        if let Some(handler) = handler {
                            let (control, payload) = frame.split_at(control_len);
                            handler(connection.node, control, payload);
                        }
                    }
                    // `read_exact` never returns `Ok(0)`: EOF surfaces as an
                    // `UnexpectedEof` error. At the header read — the frame
                    // boundary — that is the peer closing its side (FIN), not
                    // a read fault, so it maps to `PeerClosed`, keeping that
                    // variant's documented meaning reachable. (EOF mid-header
                    // is indistinguishable here and also maps to `PeerClosed`;
                    // either way the peer's side of the socket is gone.)
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                        connection.mark_down(ConnectionDownReason::PeerClosed);
                        break;
                    }
                    Err(_) => {
                        connection.mark_down(ConnectionDownReason::ReadError);
                        break;
                    }
                }
            }
        });
    }

    /// Spawn the proactive net-tick for `connection` when heartbeats are enabled.
    ///
    /// Every `interval` the task: (1) writes a [`KEEPALIVE_FRAME`] (via
    /// `write_raw`, which itself marks the link down on a write error), and (2)
    /// marks the link down via the existing connection-down path if no inbound
    /// bytes have arrived within `deadline` — catching a silently-partitioned
    /// peer that never sends a FIN/RST. The task exits once the connection is
    /// down (whether from the heartbeat, a read error, or a manual disconnect),
    /// so it does not outlive the link. No-op when heartbeats are disabled.
    fn spawn_heartbeat(&self, connection: Arc<DistConnection>) {
        let Some(config) = self.inner.heartbeat else {
            return;
        };
        self.inner
            .heartbeat_tasks_spawned
            .fetch_add(1, Ordering::Relaxed);
        self.inner.spawn_lifecycle(async move {
            let mut ticker = tokio::time::interval(config.interval);
            // The first tick fires immediately; skip it so the seeded inbound
            // clock is never compared against a zero-elapsed deadline.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if connection.is_down() {
                    break;
                }
                if connection.inbound_idle_for(config.deadline) {
                    connection.mark_down_heartbeat_timeout();
                    break;
                }
                // Best-effort keepalive: a write error already drives mark_down
                // inside write_raw, so a failure here simply ends the task on the
                // next is_down() check.
                let _ = connection.write_raw(&KEEPALIVE_FRAME).await;
            }
        });
    }

    /// Accept inbound links, bounded by the receive-residency envelope (D5).
    ///
    /// Every accepted stream reserves
    /// [`INBOUND_RESIDENCY_PER_PEER_BYTES`](super::residency::INBOUND_RESIDENCY_PER_PEER_BYTES) —
    /// one framed buffer, the worst-case residency a single peer can hold —
    /// before any work is spawned for it. When the reservation would carry
    /// inbound residency past
    /// [`INBOUND_RESIDENCY_ENVELOPE_BYTES`](super::residency::INBOUND_RESIDENCY_ENVELOPE_BYTES), the stream
    /// is DECLINED: it is dropped, which closes the TCP connection so the peer
    /// sees EOF and may redial once residency frees up. That is the same
    /// disposal this path already applies to a stream whose handshake fails or
    /// times out (`handle_accepted`), so a declined peer is indistinguishable
    /// from a refused one — and, like a refused install, it is observable as a
    /// counter ([`ConnectionManager::inbound_accepts_refused`]).
    ///
    /// The reservation is charged here rather than at registration on purpose:
    /// a burst of concurrent inbound handshakes would otherwise all pass an
    /// uncharged check and register together, overshooting the envelope. The
    /// permit travels with the stream and is released by whichever exit it
    /// meets — handshake failure, a lost install dedup, or the link's own read
    /// lifecycle ending.
    ///
    /// Scope, stated: this bounds the population the LISTENER admits, which is
    /// the population a remote party controls and the one step 4 of
    /// [`MAX_DIST_FRAME_BYTES`](crate::distribution::etf::MAX_DIST_FRAME_BYTES)'s
    /// derivation names as unbounded. Locally
    /// initiated outbound dials (`connect`) are not charged: they are
    /// enumerated by this node's own resolver and configuration, not by a peer.
    pub(super) async fn accept_loop(&self, listener: TcpListener, shutdown: Arc<Notify>) {
        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    break;
                }
                accepted = listener.accept() => {
                    let Ok((stream, peer_addr)) = accepted else {
                        continue;
                    };
                    let Some(admission) = self.inner.inbound_residency.try_admit() else {
                        // Envelope exhausted. Dropping the stream closes the
                        // TCP connection (the handshake-failure disposal), and
                        // the loop keeps accepting so the listener never wedges.
                        drop(stream);
                        continue;
                    };
                    self.handle_accepted(stream, peer_addr, admission);
                }
            }
        }
    }

    /// Run the inbound OTP handshake on an accepted stream, then register it.
    ///
    /// The handshake is asynchronous, so it is spawned onto the bound runtime via
    /// [`ConnectionManagerInner::spawn_lifecycle`](super::ConnectionManagerInner::spawn_lifecycle) — the same mechanism the
    /// read/accept lifecycle uses — so it is driven even in production where no
    /// ambient tokio runtime exists on worker threads. The handshake completes on
    /// the raw stream (2-byte length-prefixed packets) before the connection is
    /// registered and its data-frame read loop starts. On success the connection
    /// is keyed by the peer's authenticated handshake name; on failure the stream
    /// is dropped, closing the TCP connection.
    fn handle_accepted(
        &self,
        mut stream: TcpStream,
        peer_addr: SocketAddr,
        admission: InboundAdmissionPermit,
    ) {
        let manager = self.clone();
        self.inner.spawn_lifecycle(async move {
            let local = match manager.inner.handshake_node() {
                Ok(local) => local,
                Err(_) => return,
            };
            // Bound the responder so a stalled or malicious peer can never park
            // this spawned task forever; on elapse the stream is dropped, closing
            // the TCP connection (HS-1). The decider resolves a simultaneous
            // connect by the name-comparison tie-break against the local outbound
            // state (HS-3); on `nok` the responder aborts and the reciprocal
            // outbound is the survivor.
            let decider = |peer_name: &str| manager.inner.decide_inbound_status(peer_name);
            let outcome = tokio::time::timeout(
                manager.inner.handshake_timeout,
                respond_handshake_async_with(
                    &mut stream,
                    &local,
                    &manager.inner.cookie,
                    manager.inner.gen_challenge(),
                    decider,
                ),
            )
            .await;
            match outcome {
                Ok(Ok(result)) => {
                    let node = manager.inner.atom_table.intern(result.remote_name());
                    // A refused install (teardown-dup failure under fd
                    // exhaustion) drops the stream: the peer sees EOF and may
                    // redial once descriptors free up.
                    let _ = manager.register_connection(
                        node,
                        peer_addr,
                        stream,
                        LinkDirection::Inbound,
                        result.remote_creation(),
                        Some(admission),
                    );
                }
                Ok(Err(_)) | Err(_) => {
                    // The residency reservation goes with the stream: dropping
                    // `admission` here returns this peer's envelope share the
                    // moment the handshake fails or times out.
                    drop(admission);
                    drop(stream);
                }
            }
        });
    }
}
