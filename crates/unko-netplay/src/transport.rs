//! Message transport abstraction and an in-process channel pair.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};

/// Sends and receives complete netplay messages.
///
/// Implementations must preserve message boundaries: one `send` is one `recv`.
pub trait Transport {
    /// Sends one message.
    ///
    /// # Errors
    ///
    /// Returns the transport's I/O error.
    fn send(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Receives one message, or `None` when nothing is queued.
    ///
    /// # Errors
    ///
    /// Returns the transport's I/O error.
    fn recv(&mut self) -> io::Result<Option<Vec<u8>>>;
}

/// In-process transport backed by two `mpsc` channels.
#[derive(Debug)]
pub struct ChannelTransport {
    outgoing: Sender<Vec<u8>>,
    incoming: Receiver<Vec<u8>>,
}

impl Transport for ChannelTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.outgoing
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer closed"))
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.incoming.try_recv() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "peer closed"))
            }
        }
    }
}

/// Fan-out transport for a peer mesh.
///
/// `send` broadcasts one message to every peer; `recv` polls each peer in
/// order and returns the first complete message. Messages already carry the
/// sender's player tag, so the transport does not need to identify the source.
#[derive(Default)]
pub struct MeshTransport {
    peers: Vec<Box<dyn Transport>>,
}

impl MeshTransport {
    /// Wraps an existing set of peer transports.
    #[must_use]
    pub fn new(peers: Vec<Box<dyn Transport>>) -> Self {
        Self { peers }
    }

    /// Adds one peer.
    pub fn push(&mut self, peer: Box<dyn Transport>) {
        self.peers.push(peer);
    }

    /// Number of connected peers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether no peer is connected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

impl Transport for MeshTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut first_error = None;
        for peer in &mut self.peers {
            if let Err(err) = peer.send(bytes) {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        for peer in &mut self.peers {
            if let Some(bytes) = peer.recv()? {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }
}

/// Creates two connected in-process transports.
#[must_use]
pub fn channel_pair() -> (ChannelTransport, ChannelTransport) {
    let (a_tx, b_rx) = mpsc::channel();
    let (b_tx, a_rx) = mpsc::channel();
    (
        ChannelTransport {
            outgoing: a_tx,
            incoming: a_rx,
        },
        ChannelTransport {
            outgoing: b_tx,
            incoming: b_rx,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_broadcasts_and_polls_every_peer() {
        let (a, b) = channel_pair();
        let (c, d) = channel_pair();
        let mut mesh = MeshTransport::new(vec![Box::new(a), Box::new(c)]);
        let mut first = b;
        let mut second = d;
        assert!(mesh.recv().expect("empty").is_none());
        mesh.send(b"hello").expect("broadcast");
        assert_eq!(first.recv().expect("first"), Some(b"hello".to_vec()));
        assert_eq!(second.recv().expect("second"), Some(b"hello".to_vec()));
    }

    #[test]
    fn channel_pair_preserves_messages() {
        let (mut a, mut b) = channel_pair();
        assert!(b.recv().expect("recv").is_none());
        a.send(b"one").expect("send");
        a.send(b"two").expect("send");
        assert_eq!(b.recv().expect("recv"), Some(b"one".to_vec()));
        assert_eq!(b.recv().expect("recv"), Some(b"two".to_vec()));
        assert!(b.recv().expect("recv").is_none());
    }
}
