//! UDP hole punching for NetPlay.
//!
//! Both peers bind their game port, discover their public endpoint out of
//! band (UPnP/STUN/manual), and send `PUNCH` datagrams to each other. A router
//! with port-restricted filtering drops the first unsolicited datagram but
//! opens the mapping for the outbound one; the reply `PONG` then confirms that
//! traffic can flow in both directions. Game messages are queued until the
//! caller starts the session.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::Transport;

/// Control packet magic.
const PUNCH: &[u8; 8] = b"UNKOPNG1";
/// Punch acknowledgement magic.
const PONG: &[u8; 8] = b"UNKOPNG2";
/// Control packet length (magic + reserved tag).
const CONTROL_LEN: usize = 16;
/// Largest accepted datagram.
const MAX_DATAGRAM: usize = 2048;
/// Bound queued datagrams (UDP is already allowed to drop).
const MAX_PENDING: usize = 256;
/// Retry interval between punches.
const PUNCH_INTERVAL: Duration = Duration::from_millis(200);

/// One peer's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PunchPeer {
    /// Advertised (public) endpoint.
    pub endpoint: SocketAddr,
    /// A `PUNCH`/`PONG` exchange succeeded.
    pub confirmed: bool,
}

/// A non-blocking UDP socket that punches through NAT before play.
#[derive(Debug)]
pub struct HolePunchTransport {
    socket: UdpSocket,
    peers: BTreeMap<SocketAddr, bool>,
    pending: VecDeque<(SocketAddr, Vec<u8>)>,
    buffer: Vec<u8>,
    last_punch: Option<Instant>,
}

impl HolePunchTransport {
    /// Binds `address` and prepares punches for `peers`.
    ///
    /// # Errors
    ///
    /// Returns the bind/configuration error.
    pub fn bind(address: &str, peers: &[SocketAddr]) -> io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        let mut map = BTreeMap::new();
        for peer in peers {
            map.insert(*peer, false);
        }
        Ok(Self {
            socket,
            peers: map,
            pending: VecDeque::new(),
            buffer: vec![0; MAX_DATAGRAM],
            last_punch: None,
        })
    }

    /// Adds a peer endpoint (for transports built before both sides are known).
    pub fn add_peer(&mut self, endpoint: SocketAddr) {
        self.peers.entry(endpoint).or_insert(false);
    }

    /// Local socket address.
    ///
    /// # Errors
    ///
    /// Returns the socket address error.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Peer states in endpoint order.
    #[must_use]
    pub fn peers(&self) -> Vec<PunchPeer> {
        self.peers
            .iter()
            .map(|(endpoint, confirmed)| PunchPeer {
                endpoint: *endpoint,
                confirmed: *confirmed,
            })
            .collect()
    }

    /// Whether every configured peer answered.
    #[must_use]
    pub fn connected(&self) -> bool {
        !self.peers.is_empty() && self.peers.values().all(|confirmed| *confirmed)
    }

    /// Number of confirmed peers.
    #[must_use]
    pub fn confirmed(&self) -> usize {
        self.peers.values().filter(|confirmed| **confirmed).count()
    }

    /// Sends punches when the retry interval elapsed and processes replies.
    pub fn punch(&mut self) {
        let due = self
            .last_punch
            .is_none_or(|last| last.elapsed() >= PUNCH_INTERVAL);
        if due {
            self.punch_now();
        } else {
            self.drain();
        }
    }

    /// Sends one punch to every peer and processes replies.
    pub fn punch_now(&mut self) {
        self.last_punch = Some(Instant::now());
        let mut packet = Vec::with_capacity(CONTROL_LEN);
        packet.extend_from_slice(PUNCH);
        packet.extend_from_slice(&[0; 8]);
        for endpoint in self.peers.keys() {
            let _ = self.socket.send_to(&packet, endpoint);
        }
        self.drain();
    }

    /// Receives control packets and queues game messages for [`Transport::recv`].
    fn drain(&mut self) {
        loop {
            match self.socket.recv_from(&mut self.buffer) {
                Ok((size, from)) => {
                    let data = &self.buffer[..size];
                    if let Some(kind) = control_kind(data) {
                        // Only known peers are accepted (and only they get a
                        // reply), so the socket cannot be used as a reflector.
                        if let Some(confirmed) = self.peers.get_mut(&from) {
                            *confirmed = true;
                            if kind == Control::Punch {
                                let mut pong = Vec::with_capacity(CONTROL_LEN);
                                pong.extend_from_slice(PONG);
                                pong.extend_from_slice(&[0; 8]);
                                let _ = self.socket.send_to(&pong, from);
                            }
                        }
                    } else if size > 0 && self.pending.len() < MAX_PENDING {
                        self.pending.push_back((from, data.to_vec()));
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Punch,
    Pong,
}

/// Recognizes a control packet. Only configured peer addresses are accepted;
/// the tag is reserved for a future handshake.
fn control_kind(data: &[u8]) -> Option<Control> {
    if data.len() != CONTROL_LEN {
        return None;
    }
    if data[..8] == PUNCH[..] {
        Some(Control::Punch)
    } else if data[..8] == PONG[..] {
        Some(Control::Pong)
    } else {
        None
    }
}

impl HolePunchTransport {
    /// Sends one datagram to a known peer.
    ///
    /// # Errors
    ///
    /// Returns the socket error, or an error when `peer` is not configured.
    pub fn send_to(&mut self, peer: SocketAddr, bytes: &[u8]) -> io::Result<()> {
        if !self.peers.contains_key(&peer) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "unknown punch peer",
            ));
        }
        if bytes.len() > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        let sent = self.socket.send_to(bytes, peer)?;
        if sent != bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "partial punch datagram",
            ));
        }
        Ok(())
    }

    /// Receives one `(source, payload)` datagram, or `None`.
    ///
    /// # Errors
    ///
    /// Returns the socket error.
    pub fn recv_from(&mut self) -> io::Result<Option<(SocketAddr, Vec<u8>)>> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(Some(message));
        }
        self.drain();
        Ok(self.pending.pop_front())
    }
}

impl Transport for HolePunchTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        let mut first_error = None;
        for endpoint in self.peers.keys() {
            if let Err(err) = self.socket.send_to(bytes, endpoint) {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        Ok(self.recv_from()?.map(|(_, bytes)| bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// A middlebox that behaves like a port-restricted NAT between two peers:
    /// it drops traffic until both sides have sent, then relays each datagram
    /// to the other sender.
    fn restricted_relay() -> (SocketAddr, thread::JoinHandle<()>) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("relay bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("timeout");
        let address = socket.local_addr().expect("relay addr");
        let handle = thread::spawn(move || {
            let mut senders: Vec<SocketAddr> = Vec::new();
            let mut buffer = [0u8; 2048];
            for _ in 0..400 {
                match socket.recv_from(&mut buffer) {
                    Ok((size, from)) => {
                        if !senders.contains(&from) {
                            senders.push(from);
                        }
                        if senders.len() < 2 {
                            continue;
                        }
                        for target in &senders {
                            if *target != from {
                                let _ = socket.send_to(&buffer[..size], target);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        (address, handle)
    }

    #[test]
    fn loopback_peers_confirm_and_pass_messages() {
        let mut a = HolePunchTransport::bind("127.0.0.1:0", &[]).expect("a");
        let a_endpoint = a.local_addr().expect("a addr");
        let mut b = HolePunchTransport::bind("127.0.0.1:0", &[a_endpoint]).expect("b");
        let b_endpoint = b.local_addr().expect("b addr");
        a.add_peer(b_endpoint);
        assert!(!a.connected());

        for _ in 0..6 {
            a.punch_now();
            b.punch_now();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(a.connected(), "A confirmed");
        assert!(b.connected(), "B confirmed");

        a.send(b"game").expect("send");
        assert_eq!(b.recv().expect("recv"), Some(b"game".to_vec()));
        b.send(b"reply").expect("send");
        assert_eq!(a.recv().expect("recv"), Some(b"reply".to_vec()));
    }

    #[test]
    fn restricted_nat_needs_bidirectional_punches() {
        let (relay, server) = restricted_relay();
        let a = HolePunchTransport::bind("127.0.0.1:0", &[relay]).expect("a");
        let b = HolePunchTransport::bind("127.0.0.1:0", &[relay]).expect("b");
        let mut a = a;
        let mut b = b;
        // One-sided traffic is dropped by the restricted mapping.
        a.punch_now();
        for _ in 0..5 {
            a.punch_now();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!a.connected());

        // Both sides punching opens it, and retries confirm the peer.
        for _ in 0..20 {
            a.punch_now();
            b.punch_now();
            thread::sleep(Duration::from_millis(5));
            if a.connected() && b.connected() {
                break;
            }
        }
        assert!(a.connected(), "A punched through");
        assert!(b.connected(), "B punched through");
        drop(server);
    }

    #[test]
    fn control_packets_are_distinguished_from_game_data() {
        let mut punch = PUNCH.to_vec();
        punch.extend_from_slice(&[0; 8]);
        assert_eq!(control_kind(&punch), Some(Control::Punch));
        let mut pong = PONG.to_vec();
        pong.extend_from_slice(&[0; 8]);
        assert_eq!(control_kind(&pong), Some(Control::Pong));
        assert_eq!(control_kind(b"UNKOI2...."), None);
        assert_eq!(control_kind(&punch[..15]), None);
    }
}
