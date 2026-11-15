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
            ports: [1, 2, 3, 4],
        };
        a.send(&frame.encode()).expect("send");
        let bytes = b.recv().expect("recv").expect("datagram");
        assert_eq!(InputFrame::decode(&bytes), Some(frame));
        assert!(b.recv().expect("drained").is_none());
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
