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
