use std::io;
use std::sync::Arc;

use tokio::net::TcpStream;

use crate::distribution::handshake::{HandshakeError, initiate_handshake_async};

use super::{ConnectError, ConnectingGuard, ConnectionManager, DistConnection, LinkDirection};

impl ConnectionManager {
    /// Resolve `node_name`, open a TCP connection, run the OTP distribution
    /// handshake, and add the authenticated link to the active table.
    ///
    /// The connection is keyed by the name the peer advertises in the handshake
    /// — not by `node_name`/the resolver key — so identity is established by the
    /// authenticated handshake rather than by trusting the dialed address. On any
    /// handshake failure the stream is dropped (closing the TCP connection) and a
    /// [`ConnectError::Io`] is returned.
    pub async fn connect(&self, node_name: &str) -> Result<Arc<DistConnection>, ConnectError> {
        let addr = self
            .inner
            .resolver
            .resolve(node_name)
            .await
            .map_err(|_| ConnectError::ResolveFailure)?;
        let mut stream = match tokio::time::timeout(
            self.inner.connect_timeout,
            TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {
                return Err(ConnectError::ConnectionRefused);
            }
            Ok(Err(error)) => return Err(ConnectError::Io(error.to_string())),
            Err(_) => return Err(ConnectError::Timeout),
        };
        let peer_addr = stream.peer_addr().unwrap_or(addr);

        let local = self.inner.handshake_node()?;
        // Mark this peer name as having an in-flight outbound BEFORE the handshake
        // awaits, so a concurrent inbound responder can detect the simultaneous
        // case and apply the tie-break (HS-3). The guard clears the mark on every
        // exit path. The dialed `node_name` is the peer's authenticated name in a
        // by-name cluster mesh (haematite FullMesh), which is what the peer
        // advertises and what its responder compares against.
        let _connecting = ConnectingGuard::new(&self.inner, node_name);
        // Bound the whole handshake so a stalled or malicious peer can never park
        // this call forever; `connect` is now guaranteed to return within
        // handshake_timeout (HS-1). On elapse the stream is dropped, closing the
        // TCP connection.
        let result = match tokio::time::timeout(
            self.inner.handshake_timeout,
            initiate_handshake_async(
                &mut stream,
                &local,
                &self.inner.cookie,
                self.inner.gen_challenge(),
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(HandshakeError::BadStatus(status))) if status == "nok" => {
                // The peer kept the reciprocal link via the tie-break. Benign:
                // drop our stream and report a non-failure abort so the caller
                // does not retry-storm.
                return Err(ConnectError::SimultaneousAbort);
            }
            Ok(Err(error)) => return Err(ConnectError::Io(error.to_string())),
            Err(_) => return Err(ConnectError::Io(HandshakeError::Timeout.to_string())),
        };
        // Dropping the stream on the error paths above closes the TCP connection;
        // on success the authenticated remote name becomes the connection-table
        // key.
        //
        // Tie-break, install side: if our concurrent inbound responder already
        // decided to keep the reciprocal inbound link for this peer (we are the
        // lower-named node, HS-3 §3.2), retire this outbound instead of also
        // installing it. Two installs for one peer would otherwise collide in the
        // HS-2 dedup, and the loser-socket drop can tear down the peer's surviving
        // link, leaving the pair with zero links and no re-dial. Dropping the
        // stream closes this TCP connection; the reciprocal inbound is the
        // survivor, so this is a benign `SimultaneousAbort`, not a failure.
        if _connecting.is_aborted() {
            drop(stream);
            return Err(ConnectError::SimultaneousAbort);
        }
        let node = self.inner.atom_table.intern(result.remote_name());
        // `None`: the accept bound charges the population the LISTENER admits.
        // A locally initiated dial is enumerated by this node's own resolver,
        // not by a peer, so it spends no accept-side envelope.
        self.register_connection(
            node,
            peer_addr,
            stream,
            LinkDirection::Outbound,
            result.remote_creation(),
            None,
        )
        .map_err(|error| ConnectError::Io(error.to_string()))
    }
}
