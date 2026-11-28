//! Simplified TURN-style UDP relay for NetPlay.
//!
//! Clients join a password-protected room on one relay endpoint; the relay
//! forwards each DATA frame to the other members of that room. Unlike full TURN
//! there is no per-client relayed transport address, permission list or channel
//! binding: all members share the relay's UDP endpoint and are identified by
//! their source address. That is sufficient as a last-resort fallback when UPnP,
//! STUN and hole punching cannot produce a direct path, and keeps the server
//! small enough to audit.
//!
//! A room is claimed by the first successful JOIN: that peer's proof (derived
//! from the room name and a password) is stored, and every later JOIN with a
//! different proof is rejected. This closes the old hole where anyone who knew
//! or guessed the user-visible room name could join and inject datagrams.
//!
//! # Residual risk
//!
//! The relay is plaintext UDP. The proof is sent in the clear on every JOIN, so
//! anyone on the path can read it and replay it to join, and the relay does not
//! authenticate or encrypt the DATA bodies. This therefore protects rooms from
//! people who merely know (or guess) the room name; it is **not** a defence
//! against an active network attacker or an on-path observer. Use trusted peers
//! and, ideally, a private relay.
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

use sha2::{Digest, Sha256};

use crate::Transport;

/// Protocol magic. Bumped to 2 for password-authenticated rooms and wide ids;
/// v1 peers are silently ignored (their magic does not match).
const MAGIC: &[u8; 8] = b"UNKORLY2";
/// Join request: `room_token[8] + proof[32] + member_id_le[2]`.
const JOIN: u8 = 1;
/// Join acknowledgement: `member_id_le[2]`.
const JOINED: u8 = 2;
/// Relayed payload; forwarded frames carry the source id before the body.
const DATA: u8 = 3;
/// Join rejection: one reason byte.
const REJECTED: u8 = 4;
/// Leave notice: `room_token[8] + member_id_le[2]`.
const LEAVE: u8 = 5;
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
/// Broadcast member id.
const BROADCAST: u16 = 0xFFFF;
/// Rejection reasons carried by [`REJECTED`].
const REJECT_BAD_PROOF: u8 = 0;
const REJECT_ROOM_FULL: u8 = 1;
const REJECT_MALFORMED: u8 = 2;

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

/// Derives the 16-byte room key from the room name and password.
///
/// This is `SHA-256("unko-relay-v2:key:" + room + 0x00 + password)[..16]`. It
/// mixes the password into a per-room value, but it is *not* a password hash
/// suitable for storage: the relay never sees the password, only the proof
/// below, and the key is derived the same way by every client. Because the
/// relay is plaintext UDP an on-path observer still sees the proof (and could
/// derive this key if it knows the password); the key does not provide
/// confidentiality or authentication against an active attacker.
#[must_use]
pub fn room_key(room: &str, password: &str) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(b"unko-relay-v2:key:");
    hasher.update(room.as_bytes());
    hasher.update([0u8]);
    hasher.update(password.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 16];
    key.copy_from_slice(&digest[..16]);
    key
}

/// Derives the 32-byte room proof a client sends in `JOIN`.
///
/// This is `SHA-256("unko-relay-v2:proof:" + room_token(room) + room_key)[..32]`.
/// The proof lets the relay verify that a joiner knows the password without
/// ever handling the password itself, and lets it pin a room to the first
/// proof it sees. It is domain-separated from [`room_key`] by the prefix, so
/// the two inputs never produce the same bytes for the same arguments.
///
/// It protects a room only against people who know the room name but not the
/// password. The relay is plaintext UDP: an on-path observer can read the
/// proof off the wire and replay it, so this is not protection against an
/// active network attacker.
#[must_use]
pub fn room_proof(room: &str, password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"unko-relay-v2:proof:");
    hasher.update(room_token(room));
    hasher.update(room_key(room, password));
    let digest = hasher.finalize();
    let mut proof = [0u8; 32];
    proof.copy_from_slice(&digest[..32]);
    proof
}

/// Compares two proofs without an early exit on the first differing byte.
fn proofs_match(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
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

struct Room {
    /// Proof pinned by the first successful JOIN.
    proof: [u8; 32],
    /// member id -> source address.
    members: HashMap<u16, SocketAddr>,
}

fn serve(socket: &UdpSocket, stop: &AtomicBool) {
    let mut rooms: HashMap<[u8; 8], Room> = HashMap::new();
    let mut buffer = [0u8; MAX_DATAGRAM];
    while !stop.load(Ordering::Relaxed) {
        let Ok((size, from)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        let Some((kind, payload)) = decode(&buffer[..size]) else {
            continue;
        };
        match kind {
            JOIN => handle_join(socket, &mut rooms, payload, from),
            DATA => {
                // payload[..2] is the destination id (0xFFFF broadcasts); the
                // rest is the GGRS datagram. The source id is prepended to the
                // forwarded frame, so a client cannot spoof another member's
                // traffic.
                if payload.len() < 2 || payload.len() > MAX_DATAGRAM - HEADER {
                    continue;
                }
                let destination = u16::from_le_bytes([payload[0], payload[1]]);
                let body = &payload[2..];
                let Some((source_id, members)) = rooms.values().find_map(|room| {
                    room.members
                        .iter()
                        .find(|(_, address)| **address == from)
                        .map(|(id, _)| (*id, &room.members))
                }) else {
                    continue;
                };
                let mut frame = Vec::with_capacity(HEADER + 2 + body.len());
                frame.extend_from_slice(MAGIC);
                frame.push(DATA);
                frame.extend_from_slice(&source_id.to_le_bytes());
                frame.extend_from_slice(body);
                for (id, member) in members {
                    if *member == from {
                        continue;
                    }
                    if destination == BROADCAST || destination == *id {
                        let _ = socket.send_to(&frame, member);
                    }
                }
            }
            LEAVE => {
                if payload.len() != 10 {
                    continue;
                }
                let mut room = [0u8; 8];
                room.copy_from_slice(&payload[..8]);
                let member_id = u16::from_le_bytes([payload[8], payload[9]]);
                let Some(entry) = rooms.get_mut(&room) else {
                    continue;
                };
                if entry.members.get(&member_id) == Some(&from) {
                    entry.members.remove(&member_id);
                }
                if entry.members.is_empty() {
                    rooms.remove(&room);
                }
            }
            _ => {}
        }
    }
}

fn handle_join(
    socket: &UdpSocket,
    rooms: &mut HashMap<[u8; 8], Room>,
    payload: &[u8],
    from: SocketAddr,
) {
    if payload.len() != 42 {
        send_rejected(socket, from, REJECT_MALFORMED);
        return;
    }
    let mut room = [0u8; 8];
    room.copy_from_slice(&payload[..8]);
    let mut proof = [0u8; 32];
    proof.copy_from_slice(&payload[8..40]);
    let member_id = u16::from_le_bytes([payload[40], payload[41]]);

    if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
        send_rejected(socket, from, REJECT_ROOM_FULL);
        return;
    }
    if let Some(existing) = rooms.get(&room) {
        if !proofs_match(&existing.proof, &proof) {
            send_rejected(socket, from, REJECT_BAD_PROOF);
            return;
        }
    } else {
        rooms.insert(
            room,
            Room {
                proof,
                members: HashMap::new(),
            },
        );
    }
    let members = &mut rooms.get_mut(&room).expect("room was just claimed").members;
    match members.get(&member_id) {
        // A retry from the same socket is idempotent.
        Some(address) if *address == from => {}
        // The id is taken by another peer: no free slot for this source.
        Some(_) => {
            send_rejected(socket, from, REJECT_ROOM_FULL);
            return;
        }
        None if members.len() >= MAX_MEMBERS => {
            send_rejected(socket, from, REJECT_ROOM_FULL);
            return;
        }
        None => {
            members.insert(member_id, from);
        }
    }
    let ack = frame(JOINED, &member_id.to_le_bytes());
    let _ = socket.send_to(&ack, from);
}

fn send_rejected(socket: &UdpSocket, to: SocketAddr, reason: u8) {
    let _ = socket.send_to(&frame(REJECTED, &[reason]), to);
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
    proof: [u8; 32],
    member_id: u16,
    joined: bool,
    last_join: Option<Instant>,
    pending: VecDeque<(u16, Vec<u8>)>,
    scratch: Vec<u8>,
}

impl RelayTransport {
    /// Binds `local` and joins `room` on `relay` (`host:port`) with `password`.
    ///
    /// # Errors
    ///
    /// Returns an address, bind or configuration error. An empty password is
    /// rejected: an unauthenticated relay room must not exist silently.
    pub fn bind(
        local: &str,
        relay: &str,
        room: &str,
        password: &str,
        member_id: u16,
    ) -> io::Result<Self> {
        if password.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relay room password must not be empty",
            ));
        }
        let relay = relay.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{relay}: no address"))
        })?;
        let socket = UdpSocket::bind(local)?;
        socket.set_nonblocking(true)?;
        let mut transport = Self {
            socket,
            relay,
            room: room_token(room),
            proof: room_proof(room, password),
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
        let mut payload = Vec::with_capacity(42);
        payload.extend_from_slice(&self.room);
        payload.extend_from_slice(&self.proof);
        payload.extend_from_slice(&self.member_id.to_le_bytes());
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
                        REJECTED => self.joined = false,
                        DATA if payload.len() >= 2 => {
                            if self.pending.len() < MAX_PENDING {
                                let source = u16::from_le_bytes([payload[0], payload[1]]);
                                self.pending.push_back((source, payload[2..].to_vec()));
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

impl Drop for RelayTransport {
    fn drop(&mut self) {
        if !self.joined {
            return;
        }
        let mut payload = Vec::with_capacity(10);
        payload.extend_from_slice(&self.room);
        payload.extend_from_slice(&self.member_id.to_le_bytes());
        let _ = self.socket.send_to(&frame(LEAVE, &payload), self.relay);
    }
}

impl RelayTransport {
    /// Sends one datagram to a room member id (0xFFFF broadcasts).
    ///
    /// # Errors
    ///
    /// Returns the relay I/O error.
    pub fn send_to(&mut self, member: u16, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATAGRAM - HEADER - 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "netplay message is too large for one datagram",
            ));
        }
        if !self.joined {
            self.send_join();
        }
        let mut payload = Vec::with_capacity(2 + bytes.len());
        payload.extend_from_slice(&member.to_le_bytes());
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
    pub fn recv_from(&mut self) -> io::Result<Option<(u16, Vec<u8>)>> {
        self.pump();
        Ok(self.pending.pop_front())
    }
}

impl Transport for RelayTransport {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send_to(BROADCAST, bytes)
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

    fn raw_socket(relay: &str) -> UdpSocket {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("raw socket");
        socket.connect(relay).expect("connect relay");
        socket
    }

    fn join_message(room: &str, password: &str, member_id: u16) -> Vec<u8> {
        let mut payload = Vec::with_capacity(42);
        payload.extend_from_slice(&room_token(room));
        payload.extend_from_slice(&room_proof(room, password));
        payload.extend_from_slice(&member_id.to_le_bytes());
        frame(JOIN, &payload)
    }

    fn read_reply(socket: &UdpSocket) -> Option<(u8, Vec<u8>)> {
        let mut buffer = [0u8; MAX_DATAGRAM];
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok()?;
        let (size, _) = socket.recv_from(&mut buffer).ok()?;
        decode(&buffer[..size]).map(|(kind, payload)| (kind, payload.to_vec()))
    }

    #[test]
    fn room_tokens_are_stable() {
        assert_eq!(room_token("unko"), room_token("unko"));
        assert_ne!(room_token("unko"), room_token("other"));
    }

    #[test]
    fn room_keys_and_proofs_are_deterministic_and_domain_separated() {
        assert_eq!(room_key("unko", "hunter2"), room_key("unko", "hunter2"));
        assert_eq!(room_proof("unko", "hunter2"), room_proof("unko", "hunter2"));
        // Different rooms, same password.
        assert_ne!(room_key("unko", "hunter2"), room_key("other", "hunter2"));
        assert_ne!(
            room_proof("unko", "hunter2"),
            room_proof("other", "hunter2")
        );
        // Same room, different passwords.
        assert_ne!(room_key("unko", "hunter2"), room_key("unko", "hunter3"));
        assert_ne!(room_proof("unko", "hunter2"), room_proof("unko", "hunter3"));
        // Domain separation: the key bytes never equal the proof's leading
        // bytes for the same inputs.
        let key = room_key("unko", "hunter2");
        let proof = room_proof("unko", "hunter2");
        assert_ne!(&key[..], &proof[..16]);
    }

    #[test]
    fn empty_password_is_rejected() {
        let err = RelayTransport::bind("127.0.0.1:0", "127.0.0.1:1", "room", "", 0)
            .expect_err("empty password must not bind");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn frames_round_trip_and_reject_noise() {
        let message = frame(DATA, b"payload");
        assert_eq!(decode(&message), Some((DATA, &b"payload"[..])));
        assert_eq!(decode(b"short"), None);
        assert_eq!(decode(b"XXXXXXXX1data"), None);
        // The v1 magic must not decode.
        let mut old = b"UNKORLY1".to_vec();
        old.push(DATA);
        assert_eq!(decode(&old), None);
    }

    #[test]
    fn relay_forwards_between_room_members() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 0).expect("a");
        let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 1).expect("b");
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
        let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "one", "secret", 0).expect("a");
        let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "one", "secret", 1).expect("b");
        let mut c = RelayTransport::bind("127.0.0.1:0", &relay, "two", "secret", 0).expect("c");
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

    #[test]
    fn different_password_is_rejected() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let first = raw_socket(&relay);
        first
            .send(&join_message("room", "correct", 0))
            .expect("first join");
        assert_eq!(read_reply(&first), Some((JOINED, vec![0, 0])));

        let second = raw_socket(&relay);
        second
            .send(&join_message("room", "wrong", 1))
            .expect("second join");
        let (kind, payload) = read_reply(&second).expect("rejection");
        assert_eq!(kind, REJECTED);
        assert_eq!(payload, vec![REJECT_BAD_PROOF]);
    }

    #[test]
    fn malformed_join_is_rejected() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let socket = raw_socket(&relay);
        socket
            .send(&frame(JOIN, &room_token("room")))
            .expect("short join");
        let (kind, payload) = read_reply(&socket).expect("rejection");
        assert_eq!(kind, REJECTED);
        assert_eq!(payload, vec![REJECT_MALFORMED]);
    }

    #[test]
    fn data_from_a_non_member_is_dropped() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut member =
            RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 7).expect("member");
        wait_joined(&mut [&mut member]);

        let stranger = raw_socket(&relay);
        let mut payload = Vec::new();
        payload.extend_from_slice(&7u16.to_le_bytes());
        payload.extend_from_slice(b"injected");
        stranger.send(&frame(DATA, &payload)).expect("inject");

        for _ in 0..20 {
            member.pump();
            assert_eq!(member.recv().expect("recv"), None, "non-member data");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn broadcast_reaches_all_peers() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 0).expect("a");
        let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 1).expect("b");
        let mut c = RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 2).expect("c");
        wait_joined(&mut [&mut a, &mut b, &mut c]);

        a.send(b"all").expect("broadcast");
        for (name, peer) in [("b", &mut b), ("c", &mut c)] {
            let mut received = None;
            for _ in 0..100 {
                peer.pump();
                if let Some(message) = peer.recv().expect("recv") {
                    received = Some(message);
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(received, Some(b"all".to_vec()), "peer {name}");
        }
    }

    #[test]
    fn member_id_collision_is_rejected() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        let mut first =
            RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 3).expect("first");
        wait_joined(&mut [&mut first]);

        let mut second =
            RelayTransport::bind("127.0.0.1:0", &relay, "room", "secret", 3).expect("second");
        for _ in 0..40 {
            second.pump();
            assert!(!second.joined(), "colliding member id must not join");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn room_is_reclaimable_with_a_new_password_after_members_leave() {
        let server = RelayServer::spawn("127.0.0.1:0").expect("server");
        let relay = server.local_addr().to_string();
        {
            let mut a =
                RelayTransport::bind("127.0.0.1:0", &relay, "reclaim", "old", 0).expect("a");
            let mut b =
                RelayTransport::bind("127.0.0.1:0", &relay, "reclaim", "old", 1).expect("b");
            wait_joined(&mut [&mut a, &mut b]);
        }
        // Dropping the transports sends LEAVE; the relay drops the empty room,
        // so a new password can claim it.
        let mut c =
            RelayTransport::bind("127.0.0.1:0", &relay, "reclaim", "new", 0).expect("reclaimer");
        wait_joined(&mut [&mut c]);
        assert!(
            c.joined(),
            "room must be reclaimable after all members leave"
        );
    }
}
