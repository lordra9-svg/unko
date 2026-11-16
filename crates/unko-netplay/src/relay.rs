//! Simplified TURN-style UDP relay for NetPlay.
//!
//! Clients join a room on one relay endpoint; the relay forwards each DATA
//! frame to the other members of that room. Unlike full TURN there is no
//! per-client relayed transport address, permission list or channel binding:
//! all members share the relay's UDP endpoint and are identified by their
//! source address. That is sufficient as a last-resort fallback when UPnP,
//! STUN and hole punching cannot produce a direct path, and keeps the server
//! small enough to audit.
//!
//! The server can run inside a test or as the `unko-relay` binary:
//! `unko-relay --bind 0.0.0.0:3478`.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::Transport;

/// Protocol magic.
const MAGIC: &[u8; 8] = b"UNKORLY1";
/// Join request.
const JOIN: u8 = 1;
/// Join acknowledgement.
const JOINED: u8 = 2;
/// Relayed payload.
const DATA: u8 = 3;
/// Header length.
const HEADER: usize = 9;
/// Largest accepted datagram.
const MAX_DATAGRAM: usize = 2048;
/// Join retry interval.
const JOIN_INTERVAL: Duration = Duration::from_millis(300);
/// Bound per-client queued datagrams (UDP is already allowed to drop).
const MAX_PENDING: usize = 256;
/// Bound relay rooms and members to prevent memory exhaustion.
const MAX_ROOMS: usize = 64;
const MAX_MEMBERS: usize = 16;

/// Stable 8-byte room token derived from a user-visible room name.
#[must_use]
pub fn room_token(room: &str) -> [u8; 8] {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in room.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.to_be_bytes()
}

/// A running relay server.
pub struct RelayServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RelayServer {
    /// Binds and starts a relay in a background thread.
    ///
    /// # Errors
    ///
    /// Returns the bind/configuration error.
    pub fn spawn(bind: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let address = socket.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = thread::spawn(move || serve(&socket, &flag));
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }

    /// Relay endpoint clients connect to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Stops the thread and waits for it.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RelayServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn serve(socket: &UdpSocket, stop: &AtomicBool) {
    // room -> (member id -> source address)
    let mut rooms: HashMap<[u8; 8], HashMap<u8, SocketAddr>> = HashMap::new();
    let mut buffer = [0u8; MAX_DATAGRAM];
    while !stop.load(Ordering::Relaxed) {
        let Ok((size, from)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        let Some((kind, payload)) = decode(&buffer[..size]) else {
            continue;
        };
        match kind {
            JOIN => {
                if payload.len() != 9 {
                    continue;
                }
                let mut room = [0u8; 8];
                room.copy_from_slice(&payload[..8]);
                let member_id = payload[8];
                if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
                    continue;
                }
                let members = rooms.entry(room).or_default();
                if !members.contains_key(&member_id) && members.len() >= MAX_MEMBERS {
                    continue;
                }
                members.insert(member_id, from);
                let mut ack = Vec::with_capacity(HEADER + 1);
                ack.extend_from_slice(MAGIC);
                ack.push(JOINED);
                ack.push(member_id);
                let _ = socket.send_to(&ack, from);
            }
            DATA => {
                // payload[0] is the destination member id (0xFF broadcasts);
                // the rest is the GGRS datagram. The source id is prepended on
                // the forwarded frame, so a client cannot spoof another
                // member's traffic.
                if payload.is_empty() {
                    continue;
                }
                let destination = payload[0];
                let body = &payload[1..];
                let Some((source_id, members)) = rooms.iter().find_map(|(_, members)| {
                    members
                        .iter()
                        .find(|(_, address)| **address == from)
                        .map(|(id, _)| (*id, members))
                }) else {
                    continue;
                };
                let mut frame = Vec::with_capacity(HEADER + 1 + body.len());
                frame.extend_from_slice(MAGIC);
                frame.push(DATA);
                frame.push(source_id);
                frame.extend_from_slice(body);
                for (id, member) in members {
                    if *member == from {
                        continue;
                    }
                    if destination == 0xFF || destination == *id {
                        let _ = socket.send_to(&frame, member);
                    }
                }
            }
            _ => {}
        }
    }
}

fn decode(bytes: &[u8]) -> Option<(u8, &[u8])> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return None;
    }
    Some((bytes[8], &bytes[HEADER..]))
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(kind);
    out.extend_from_slice(payload);
    out
}

/// Client side of the relay.
#[derive(Debug)]
pub struct RelayTransport {
    socket: UdpSocket,
    relay: SocketAddr,
    room: [u8; 8],
    member_id: u8,
    joined: bool,
    last_join: Option<Instant>,
    pending: VecDeque<(u8, Vec<u8>)>,
    scratch: Vec<u8>,
}

impl RelayTransport {
    /// Binds `local` and joins `room` on `relay` (`host:port`).
    ///
    /// # Errors
    ///
    /// Returns an address, bind or configuration error.
    pub fn bind(local: &str, relay: &str, room: &str, member_id: u8) -> io::Result<Self> {
        let relay = relay.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{relay}: no address"))
        })?;
        let socket = UdpSocket::bind(local)?;
        socket.set_nonblocking(true)?;
        let mut transport = Self {
            socket,
            relay,
            room: room_token(room),
            member_id,
            joined: false,
            last_join: None,
            pending: VecDeque::new(),
            scratch: vec![0; MAX_DATAGRAM],
        };
        transport.send_join();
        Ok(transport)
    }

    /// Local socket address.
    ///
    /// # Errors
    ///
    /// Returns the socket address error.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Whether the relay acknowledged the join.
    #[must_use]
    pub const fn joined(&self) -> bool {
        self.joined
    }

    /// Retries the join and processes control traffic.
    pub fn pump(&mut self) {
        if !self.joined
            && self
                .last_join
                .is_none_or(|last| last.elapsed() >= JOIN_INTERVAL)
        {
            self.send_join();
        }
        self.drain();
    }

    fn send_join(&mut self) {
        self.last_join = Some(Instant::now());
        let mut payload = Vec::with_capacity(9);
        payload.extend_from_slice(&self.room);
        payload.push(self.member_id);
        let message = frame(JOIN, &payload);
        let _ = self.socket.send_to(&message, self.relay);
    }

    fn drain(&mut self) {
        loop {
            match self.socket.recv_from(&mut self.scratch) {
                Ok((size, from)) if from == self.relay => {
                    let Some((kind, payload)) = decode(&self.scratch[..size]) else {
                        continue;
                    };
                    match kind {
                        JOINED => self.joined = true,
                        DATA if !payload.is_empty() => {
                            if self.pending.len() < MAX_PENDING {
                                self.pending.push_back((payload[0], payload[1..].to_vec()));
                            }
                        }
                        DATA => {}
                        _ => {}
                    }
                }
                Ok(_) => {}
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

impl RelayTransport {
    /// Sends one datagram to a room member id (0xFF broadcasts).
    ///
    /// # Errors
    ///
    /// Returns the relay I/O error.
    pub fn send_to(&mut self, member: u8, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATAGRAM - 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        if !self.joined {
            self.send_join();
        }
        let mut payload = Vec::with_capacity(1 + bytes.len());
        payload.push(member);
        payload.extend_from_slice(bytes);
        let message = frame(DATA, &payload);
        let sent = self.socket.send_to(&message, self.relay)?;
        if sent != message.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "partial relay datagram",
            ));
        }
        Ok(())
    }

    /// Receives one `(source member id, payload)` datagram, or `None`.
    ///
    /// # Errors
    ///
    /// Returns the relay I/O error.
    pub fn recv_from(&mut self) -> io::Result<Option<(u8, Vec<u8>)>> {
        self.pump();
        Ok(self.pending.pop_front())
    }
}

impl Transport for RelayTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send_to(0xFF, bytes)
    }

    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        Ok(self.recv_from()?.map(|(_, bytes)| bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_joined(clients: &mut [&mut RelayTransport]) {
        for _ in 0..100 {
            for client in clients.iter_mut() {
                client.pump();
            }
            if clients.iter().all(|client| client.joined()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("relay join timed out");
    }

    #[test]
    fn room_tokens_are_stable() {
        assert_eq!(room_token("unko"), room_token("unko"));
        assert_ne!(room_token("unko"), room_token("other"));
    }

    #[test]
    fn frames_round_trip_and_reject_noise() {
        let message = frame(DATA, b"payload");
        assert_eq!(decode(&message), Some((DATA, &b"payload"[..])));
        assert_eq!(decode(b"short"), None);
        assert_eq!(decode(b"XXXXXXXX1data"), None);
    }

    #[test]
    fn relay_forwards_between_room_members() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "room", 0).expect("a");
        let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "room", 1).expect("b");
        wait_joined(&mut [&mut a, &mut b]);
        assert!(a.joined() && b.joined());

        a.send(b"game").expect("send");
        let mut received = None;
        for _ in 0..100 {
            b.pump();
            if let Some(message) = b.recv().expect("recv") {
                received = Some(message);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(received, Some(b"game".to_vec()));

        b.send(b"reply").expect("send");
        let mut received = None;
        for _ in 0..100 {
            a.pump();
            if let Some(message) = a.recv().expect("recv") {
                received = Some(message);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(received, Some(b"reply".to_vec()));
    }

    #[test]
    fn rooms_are_isolated() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "one", 0).expect("a");
        let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "one", 1).expect("b");
        let mut c = RelayTransport::bind("127.0.0.1:0", &relay, "two", 0).expect("c");
        wait_joined(&mut [&mut a, &mut b, &mut c]);

        a.send(b"a-data").expect("send");
        for _ in 0..20 {
            b.pump();
            c.pump();
            let _ = b.recv().expect("b");
            assert_eq!(c.recv().expect("c"), None, "other room must stay silent");
            thread::sleep(Duration::from_millis(5));
        }
    }
}
