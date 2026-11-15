//! Non-blocking UDP transport for netplay messages.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use crate::Transport;

/// Largest datagram accepted; input frames are a few bytes.
const MAX_DATAGRAM: usize = 2048;

/// A connected, non-blocking UDP transport.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    buffer: Vec<u8>,
}

impl UdpTransport {
    /// Binds a local UDP socket and enables non-blocking mode.
    ///
    /// # Errors
    ///
    /// Returns the socket bind or configuration error.
    pub fn bind(address: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            buffer: vec![0; MAX_DATAGRAM],
        })
    }

    /// Restricts the transport to one peer address.
    ///
    /// # Errors
    ///
    /// Returns the socket connect error.
    pub fn connect(&mut self, peer: &str) -> io::Result<()> {
        self.socket.connect(peer)
    }

    /// Local socket address, including the OS-assigned port.
    ///
    /// # Errors
    ///
    /// Returns the socket address error.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Transport for UdpTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        let sent = self.socket.send(bytes)?;
        if sent != bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "partial UDP datagram",
            ));
        }
        Ok(())
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.socket.recv(&mut self.buffer) {
            Ok(size) => Ok(Some(self.buffer[..size].to_vec())),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }
}

/// A non-blocking UDP mesh: one socket sending to several peers.
///
/// Messages carry the sender's player tag, so the receiver does not need to
/// map datagrams back to addresses.
#[derive(Debug)]
pub struct UdpMesh {
    socket: UdpSocket,
    peers: Vec<SocketAddr>,
    buffer: Vec<u8>,
}

impl UdpMesh {
    /// Binds the shared local socket and enables non-blocking mode.
    ///
    /// # Errors
    ///
    /// Returns the socket bind or configuration error.
    pub fn bind(address: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            peers: Vec::new(),
            buffer: vec![0; MAX_DATAGRAM],
        })
    }

    /// Adds a peer address.
    ///
    /// # Errors
    ///
    /// Returns an error for an unparseable address.
    pub fn add_peer(&mut self, peer: &str) -> io::Result<()> {
        let address: SocketAddr = peer
            .parse()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, format!("{peer}: {err}")))?;
        self.peers.push(address);
        Ok(())
    }

    /// Number of configured peers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether no peer is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Local socket address, including the OS-assigned port.
    ///
    /// # Errors
    ///
    /// Returns the socket address error.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Transport for UdpMesh {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        let mut first_error = None;
        for peer in &self.peers {
            if let Err(err) = self.socket.send_to(bytes, *peer) {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.socket.recv_from(&mut self.buffer) {
            Ok((size, _from)) => Ok(Some(self.buffer[..size].to_vec())),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputFrame;

    #[test]
    fn loopback_transports_exchange_a_frame() {
        let mut a = UdpTransport::bind("127.0.0.1:0").expect("bind a");
        let mut b = UdpTransport::bind("127.0.0.1:0").expect("bind b");
        let a_addr = a.local_addr().expect("a addr");
        let b_addr = b.local_addr().expect("b addr");
        a.connect(&b_addr.to_string()).expect("connect a");
        b.connect(&a_addr.to_string()).expect("connect b");

        assert!(b.recv().expect("empty").is_none());
        let frame = InputFrame {
            frame: 7,
            player: 2,
            input: 0x0203,
        };
        a.send(&frame.encode()).expect("send");
        let bytes = b.recv().expect("recv").expect("datagram");
        assert_eq!(InputFrame::decode(&bytes), Some(frame));
        assert!(b.recv().expect("drained").is_none());
    }

    #[test]
    fn udp_mesh_reaches_every_peer() {
        let mut hub = UdpMesh::bind("127.0.0.1:0").expect("hub");
        let mut first = UdpTransport::bind("127.0.0.1:0").expect("first");
        let mut second = UdpTransport::bind("127.0.0.1:0").expect("second");
        hub.add_peer(&first.local_addr().expect("addr").to_string())
            .expect("peer 1");
        hub.add_peer(&second.local_addr().expect("addr").to_string())
            .expect("peer 2");
        first
            .connect(&hub.local_addr().expect("hub addr").to_string())
            .expect("connect 1");
        second
            .connect(&hub.local_addr().expect("hub addr").to_string())
            .expect("connect 2");
        let frame = InputFrame {
            frame: 3,
            player: 1,
            input: 0x0102,
        };
        hub.send(&frame.encode()).expect("broadcast");
        for peer in [&mut first, &mut second] {
            let bytes = peer.recv().expect("recv").expect("datagram");
            assert_eq!(InputFrame::decode(&bytes), Some(frame));
        }
    }

    #[test]
    fn rejects_oversized_messages() {
        let mut transport = UdpTransport::bind("127.0.0.1:0").expect("bind");
        transport
            .connect("127.0.0.1:9")
            .expect("connect accepts any address");
        let error = transport.send(&vec![0u8; MAX_DATAGRAM + 1]);
        assert!(error.is_err());
    }
}
