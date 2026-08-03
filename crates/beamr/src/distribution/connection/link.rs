use std::fmt;
use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

use crate::atom::Atom;

use super::{
    ConnectionDownReason, ConnectionGeneration, ConnectionManagerInner, DEFAULT_HEARTBEAT_DEADLINE,
    DEFAULT_HEARTBEAT_INTERVAL, LinkDirection,
};

/// Error returned while creating an outbound distribution TCP connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectError {
    /// The node resolver could not turn the node name into a socket address.
    ResolveFailure,
    /// The remote address refused the TCP connection.
    ConnectionRefused,
    /// Resolution succeeded but the TCP connect did not finish before the configured timeout.
    Timeout,
    /// The responder answered the simultaneous-connect tie-break with `nok`: the
    /// peer is keeping the reciprocal (its own outbound) link, so this outbound is
    /// a benign abort, NOT a failure. The caller should treat the pair as
    /// connected via the reciprocal link and must not retry-storm (HS-3, §3.2).
    SimultaneousAbort,
    /// TCP connection failed for an I/O reason other than refusal.
    Io(String),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResolveFailure => formatter.write_str("distribution node resolution failed"),
            Self::ConnectionRefused => formatter.write_str("distribution TCP connection refused"),
            Self::Timeout => formatter.write_str("distribution TCP connection timed out"),
            Self::SimultaneousAbort => formatter
                .write_str("distribution outbound aborted by simultaneous-connect tie-break"),
            Self::Io(error) => write!(formatter, "distribution TCP connection failed: {error}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// Proactive net-tick (heartbeat) configuration for idle distribution links.
///
/// When enabled, each connection runs a periodic task that (1) writes a
/// `KEEPALIVE_FRAME` so the peer's read loop refreshes its liveness clock, and
/// (2) marks the link down if no inbound bytes have arrived within `deadline`.
/// `deadline` MUST exceed `interval` (by a comfortable margin) so a healthy
/// peer's own keepalives always refresh liveness before the deadline and no
/// healthy idle link is spuriously downed. Disabled by default.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HeartbeatConfig {
    /// How often an idle link emits a keepalive and checks the deadline.
    pub interval: Duration,
    /// Inbound-idle duration after which the link is marked down.
    pub deadline: Duration,
}

impl HeartbeatConfig {
    /// Sane production defaults: 15s tick, 45s deadline.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            interval: DEFAULT_HEARTBEAT_INTERVAL,
            deadline: DEFAULT_HEARTBEAT_DEADLINE,
        }
    }
}

/// Active distribution TCP connection shared by distribution subsystems.
pub struct DistConnection {
    pub(super) node: Atom,
    peer_addr: SocketAddr,
    /// `Some` while the write direction is open; taken (sending FIN — dropping
    /// an `OwnedWriteHalf` shuts down the write direction of the shared stream)
    /// by `mark_down`/`write_raw` once the connection is down, so a retained
    /// `Arc<DistConnection>` cannot keep the socket's write half alive after
    /// teardown (spec §3.6 connection-complete shutdown).
    writer: Mutex<Option<OwnedWriteHalf>>,
    /// CLOEXEC duplicate of the socket's fd, owned by this connection, so
    /// `mark_down` can `shutdown(2)` the socket WITHOUT the writer mutex:
    /// `shutdown` acts on the socket (shared by every descriptor), the wire
    /// closes (FIN) and any write blocked on it errors immediately, even while
    /// a blocked or aborted-mid-poll write holds the writer mutex. Owning a
    /// dup (not the raw fd) makes this immune to fd reuse; taken (descriptor
    /// RELEASED) by the first `mark_down`, so a retained
    /// `Arc<DistConnection>` holds no dead descriptor after teardown. Costs
    /// one fd per live connection (§6 resource-budget line, routed to the
    /// pair). Duplication failure at construction REFUSES the connection — a
    /// connection that cannot guarantee teardown is not installed. The mutex
    /// is uncontended in practice (construction and one mark_down).
    #[cfg(unix)]
    socket_fd: StdMutex<Option<OwnedFd>>,
    pub(super) down: AtomicBool,
    manager: Weak<ConnectionManagerInner>,
    /// Monotonic base for this connection's inbound-liveness clock.
    created_at: Instant,
    /// Milliseconds since `created_at` at which inbound bytes were last observed
    /// by the read loop (any data frame OR keepalive). Read by the net-tick to
    /// detect a silently-partitioned peer via a missed deadline.
    last_inbound_millis: AtomicU64,
    /// Fired by `mark_down` so the read loop wakes and exits promptly instead of
    /// blocking on `read_exact` until the peer happens to close. This drops the
    /// read half (closing the socket) the moment the link is retired — e.g. when
    /// a simultaneous-connect canonical winner displaces this non-canonical link,
    /// so its socket cannot linger as an orphaned half-link.
    pub(super) shutdown: Arc<Notify>,
    /// Which side opened this link. Read by the install-time dedup so a
    /// canonical-direction newcomer can displace a non-canonical incumbent during
    /// a simultaneous connect, while a lone re-dial (any direction, no canonical
    /// incumbent) still installs normally.
    pub(super) direction: LinkDirection,
    /// Session generation this socket serves (immutable; inherited across
    /// simultaneous-connect displacement).
    generation: ConnectionGeneration,
    /// Peer incarnation from the authenticated handshake (0 = handshake-less
    /// test helper sentinel).
    peer_creation: u32,
    /// Reason recorded by `mark_down` BEFORE the down flag flips, so any
    /// observer of `is_down() == true` reads `Some` (set → swap(AcqRel) gives
    /// happens-before). First-set-wins on a `mark_down` race: both reasons are
    /// genuine.
    pub(super) down_reason: OnceLock<ConnectionDownReason>,
}

/// A stream paired with its pre-created teardown dup: the fallible
/// duplication happens BEFORE the connection-table entry lock, so the install
/// arms stay infallible and a dup failure refuses the connection outright
/// (spec §3.6 — a connection that cannot guarantee mutex-independent closure
/// is not installed).
pub(super) struct PreparedSocket {
    stream: TcpStream,
    /// Atomically-CLOEXEC dup of the socket fd — the authenticated
    /// distribution socket must not leak into spawned child processes.
    #[cfg(unix)]
    teardown_fd: OwnedFd,
}

impl PreparedSocket {
    pub(super) fn prepare(stream: TcpStream) -> io::Result<Self> {
        #[cfg(all(test, unix))]
        if FAIL_TEARDOWN_DUP_FOR_TEST.with(std::cell::Cell::get) {
            return Err(io::Error::other("teardown dup failure injected"));
        }
        #[cfg(unix)]
        let teardown_fd = rustix::io::fcntl_dupfd_cloexec(&stream, 0).map_err(io::Error::from)?;
        Ok(Self {
            stream,
            #[cfg(unix)]
            teardown_fd,
        })
    }
}

// Test-only fault injection for `PreparedSocket::prepare`: thread-local so a
// parallel test run cannot poison unrelated registrations.
#[cfg(all(test, unix))]
thread_local! {
    pub(crate) static FAIL_TEARDOWN_DUP_FOR_TEST: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

impl DistConnection {
    /// Split a [`PreparedSocket`] into a connection (owning the write half
    /// plus the pre-created teardown dup) and its read half.
    pub(super) fn new(
        node: Atom,
        peer_addr: SocketAddr,
        socket: PreparedSocket,
        manager: Weak<ConnectionManagerInner>,
        direction: LinkDirection,
        generation: ConnectionGeneration,
        peer_creation: u32,
    ) -> (Self, OwnedReadHalf) {
        #[cfg(unix)]
        let teardown_fd = socket.teardown_fd;
        let (read_half, writer) = socket.stream.into_split();
        let connection = Self {
            node,
            peer_addr,
            writer: Mutex::new(Some(writer)),
            #[cfg(unix)]
            socket_fd: StdMutex::new(Some(teardown_fd)),
            down: AtomicBool::new(false),
            manager,
            created_at: Instant::now(),
            last_inbound_millis: AtomicU64::new(0),
            shutdown: Arc::new(Notify::new()),
            direction,
            generation,
            peer_creation,
            down_reason: OnceLock::new(),
        };
        (connection, read_half)
    }

    /// Test-only visibility for the teardown dup: `Some(is_cloexec)` while the
    /// dup is held, `None` once released by `mark_down`.
    #[cfg(all(test, unix))]
    pub(crate) fn teardown_fd_cloexec(&self) -> Option<bool> {
        self.socket_fd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|fd| {
                rustix::io::fcntl_getfd(fd)
                    .map(|flags| flags.contains(rustix::io::FdFlags::CLOEXEC))
                    .unwrap_or(false)
            })
    }

    /// Record that inbound bytes were just observed, refreshing liveness.
    pub(super) fn note_inbound_activity(&self) {
        let elapsed = u64::try_from(self.created_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_inbound_millis.store(elapsed, Ordering::Release);
    }

    /// Whether no inbound bytes have been observed for at least `deadline`.
    pub(super) fn inbound_idle_for(&self, deadline: Duration) -> bool {
        let last = self.last_inbound_millis.load(Ordering::Acquire);
        let now = u64::try_from(self.created_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let deadline_millis = u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX);
        now.saturating_sub(last) >= deadline_millis
    }

    /// Mark this connection down because the net-tick observed no inbound
    /// liveness within the deadline (silent partition). Drives the same
    /// connection-down path a read error would.
    pub(super) fn mark_down_heartbeat_timeout(self: &Arc<Self>) {
        self.mark_down(ConnectionDownReason::HeartbeatTimeout);
    }

    /// Node-name atom used as this connection's table key.
    #[must_use]
    pub fn node(&self) -> Atom {
        self.node
    }

    /// TCP peer address for diagnostics and tests.
    #[must_use]
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Session generation this socket serves (immutable; inherited across
    /// simultaneous-connect displacement).
    #[must_use]
    pub fn generation(&self) -> ConnectionGeneration {
        self.generation
    }

    /// Peer incarnation from the authenticated handshake (0 = handshake-less
    /// test helper sentinel).
    #[must_use]
    pub fn peer_creation(&self) -> u32 {
        self.peer_creation
    }

    /// Test-only: flip the down flag WITHOUT running the reap path, holding a
    /// down-but-unreaped entry in the table (the HS-4 re-dial race window that
    /// normally lasts only between `mark_down` and `connection_down`).
    #[cfg(test)]
    pub(crate) fn force_down_without_reap(&self) {
        self.down.store(true, Ordering::Release);
    }

    /// Return true after this connection has observed a terminal read/write failure.
    #[must_use]
    pub fn is_down(&self) -> bool {
        self.down.load(Ordering::Acquire)
    }

    /// Write raw bytes to the connection and report write-side failures to the manager.
    ///
    /// This is a transport lifecycle seam only; message encoding/framing remains owned by B-117.
    pub async fn write_raw(self: &Arc<Self>, bytes: &[u8]) -> io::Result<()> {
        let result = {
            let mut writer = self.writer.lock().await;
            let result = match writer.as_mut() {
                Some(writer) => writer.write_all(bytes).await,
                // Write half already taken by teardown: the connection is down.
                None => Err(io::Error::from(io::ErrorKind::NotConnected)),
            };
            // Close-under-the-held-lock when the connection went down while this
            // write held it: `mark_down`'s own `try_lock` close is skipped in
            // that interleaving, and this check runs strictly after the down
            // store (Acquire/Release), so one of the two paths always takes the
            // half. (A write future aborted at runtime-join drops its lock
            // without reaching here — that residual half closes when the last
            // `Arc<DistConnection>` drops, the pre-teardown behavior.)
            if self.is_down() {
                writer.take();
            }
            result
        };
        if result.is_err() {
            self.mark_down(ConnectionDownReason::WriteError);
        }
        result
    }

    /// Mark this connection down because a write exceeded its deadline.
    ///
    /// The outbound sender's drain bounds each `write_raw` with a timeout so a
    /// wedged peer cannot stall propagation for the whole cluster. On timeout the
    /// write future is dropped without `write_raw` observing a failure, so the
    /// drain calls this to drive the same connection-down path (hook + remote
    /// purge) a genuine write error would. Idempotent via the inner `mark_down`.
    pub fn mark_down_write_timeout(self: &Arc<Self>) {
        self.mark_down(ConnectionDownReason::WriteTimeout);
    }

    /// Mark this connection down because the must-deliver control lane
    /// overflowed against it.
    ///
    /// Called by `DistSender::enqueue_control` when the bounded control lane is
    /// full: a peer that cannot absorb that many pending LINK/EXIT controls is
    /// effectively down (DC-1), so instead of dropping the control silently the
    /// pinned connection is torn down and the connection-down hook's
    /// noconnection backstop supplies the coarsened signals. Also the sink for
    /// control encode failures (DC-1 has no silent arm). Idempotent via the
    /// inner `mark_down`.
    pub(crate) fn mark_down_control_overflow(self: &Arc<Self>) {
        self.mark_down(ConnectionDownReason::ControlOverflow);
    }

    pub(super) fn mark_down(self: &Arc<Self>, reason: ConnectionDownReason) {
        let _ = self.down_reason.set(reason);
        if self.down.swap(true, Ordering::AcqRel) {
            return;
        }
        // Wake the read loop so it exits and drops its read half (closing the
        // socket) without waiting for the peer to close first.
        self.shutdown.notify_waiters();
        // Close the WIRE now, independent of the writer mutex: `shutdown(2)` on
        // our owned dup acts on the socket itself — FIN to the peer, and any
        // write blocked on this socket (a wedged peer holding the mutex, or a
        // write about to be aborted mid-poll) errors immediately instead of
        // holding the connection open. This is the §3.6 closure guarantee; the
        // half-takes below are resource release, not the correctness mechanism.
        // The dup is TAKEN here (descriptor released at once) — a retained
        // `Arc<DistConnection>` must not hold a dead fd until it drops.
        #[cfg(unix)]
        if let Some(fd) = self
            .socket_fd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            // ENOTCONN (already reset by the peer) is fine — the wire is down.
            let _ = rustix::net::shutdown(&fd, rustix::net::Shutdown::Both);
        }
        // Release the write half when uncontended; a write holding the lock has
        // just been errored out by the socket shutdown and its own post-write
        // `is_down` check (ordered after our `down.swap` per Acquire/Release)
        // takes the half on its way out.
        if let Ok(mut writer) = self.writer.try_lock() {
            writer.take();
        }
        if let Some(manager) = self.manager.upgrade() {
            manager.connection_down(self.node, self, reason);
        }
    }
}

/// Handle for a running inbound accept loop.
pub struct AcceptHandle {
    pub(super) local_addr: SocketAddr,
    pub(super) shutdown: Arc<Notify>,
    pub(super) task: JoinHandle<()>,
}

impl AcceptHandle {
    /// The address actually bound by the TCP listener.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Ask the accept loop to stop. The task exits asynchronously.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }

    /// Return true if the accept task has completed.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for AcceptHandle {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
        self.task.abort();
    }
}
