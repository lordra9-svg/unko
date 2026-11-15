//! Minimal STUN binding client (RFC 5389) for NetPlay.
//!
//! Sends one Binding Request to a STUN server and reads the reflexive
//! transport address from `XOR-MAPPED-ADDRESS` (or legacy `MAPPED-ADDRESS`).
//! The address is only a discovery aid: without a relay or hole punching the
//! router must still allow the port.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Binding Request message type.
const BINDING_REQUEST: u16 = 0x0001;
/// Binding Success Response message type.
const BINDING_RESPONSE: u16 = 0x0101;
/// RFC 5389 magic cookie.
const MAGIC_COOKIE: u32 = 0x2112_A442;
/// `XOR-MAPPED-ADDRESS` attribute type.
const ATTR_XOR_MAPPED: u16 = 0x0020;
/// Legacy `MAPPED-ADDRESS` attribute type.
const ATTR_MAPPED: u16 = 0x0001;
/// Header length in bytes.
const HEADER_LEN: usize = 20;

/// STUN failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StunError {
    /// Socket or address-resolution failure.
    Io(String),
    /// No matching response arrived before the timeout.
    Timeout,
    /// The response was malformed or not a binding success.
    Protocol(String),
}

impl std::fmt::Display for StunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "STUN I/O error: {message}"),
            Self::Timeout => write!(formatter, "STUN server did not answer"),
            Self::Protocol(message) => write!(formatter, "STUN protocol error: {message}"),
        }
    }
}

impl std::error::Error for StunError {}

/// Queries `server` for this host's reflexive UDP address.
///
/// `server` is `host:port` (for example `stun.l.google.com:19302`).
///
/// # Errors
///
/// Returns [`StunError::Timeout`] when no valid response arrives, or an I/O or
/// protocol error.
pub fn public_address(server: &str, timeout: Duration) -> Result<SocketAddr, StunError> {
    let address = server
        .to_socket_addrs()
        .map_err(|err| StunError::Io(format!("{server}: {err}")))?
        .next()
        .ok_or_else(|| StunError::Io(format!("{server}: no address")))?;
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|err| StunError::Io(err.to_string()))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|err| StunError::Io(err.to_string()))?;
    let transaction = transaction_id();
    let request = binding_request(transaction);
    socket
        .send_to(&request, address)
        .map_err(|err| StunError::Io(format!("{address}: {err}")))?;

    let mut buffer = [0u8; 1500];
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(StunError::Timeout);
        }
        let _ = socket.set_read_timeout(Some(remaining));
        match socket.recv_from(&mut buffer) {
            Ok((size, _)) => {
                if let Ok(address) = parse_response(&buffer[..size], transaction) {
                    return Ok(address);
                }
                // Ignore unrelated datagrams until the deadline.
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(StunError::Timeout);
            }
            Err(err) => return Err(StunError::Io(err.to_string())),
        }
    }
}

fn binding_request(transaction: [u8; 12]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(&transaction);
    out
}

fn transaction_id() -> [u8; 12] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let mix = nanos
        ^ (u64::from(std::process::id()) << 32)
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&mix.to_be_bytes());
    id[8..].copy_from_slice(&std::process::id().to_be_bytes());
    id
}

/// Parses one Binding Success Response.
///
/// Sandboxed parsers keep this testable without a network.
pub fn parse_response(bytes: &[u8], transaction: [u8; 12]) -> Result<SocketAddr, StunError> {
    if bytes.len() < HEADER_LEN {
        return Err(StunError::Protocol("short header".into()));
    }
    let kind = u16::from_be_bytes([bytes[0], bytes[1]]);
    if kind != BINDING_RESPONSE {
        return Err(StunError::Protocol(format!("message type {kind:#06X}")));
    }
    let length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if bytes.len() < HEADER_LEN + length {
        return Err(StunError::Protocol("truncated attributes".into()));
    }
    let cookie = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if cookie != MAGIC_COOKIE {
        return Err(StunError::Protocol("bad magic cookie".into()));
    }
    if bytes[8..20] != transaction {
        return Err(StunError::Protocol("transaction mismatch".into()));
    }

    let mut mapped = None;
    let mut legacy = None;
    let mut offset = HEADER_LEN;
    let end = HEADER_LEN + length;
    while offset + 4 <= end {
        let attr = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let attr_len = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
        let value_start = offset + 4;
        let value_end = value_start + attr_len;
        if value_end > end {
            return Err(StunError::Protocol("attribute overruns message".into()));
        }
        let value = &bytes[value_start..value_end];
        match attr {
            ATTR_XOR_MAPPED => mapped = decode_address(value, true).or(mapped),
            ATTR_MAPPED => legacy = decode_address(value, false).or(legacy),
            _ => {}
        }
        offset = value_start + attr_len.div_ceil(4) * 4;
    }
    mapped
        .or(legacy)
        .ok_or_else(|| StunError::Protocol("no mapped address".into()))
}

fn decode_address(value: &[u8], xor: bool) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let family = value[1];
    let port_raw = u16::from_be_bytes([value[2], value[3]]);
    let port = if xor {
        port_raw ^ (MAGIC_COOKIE >> 16) as u16
    } else {
        port_raw
    };
    match family {
        0x01 if value.len() >= 8 => {
            let mut octets = [value[4], value[5], value[6], value[7]];
            if xor {
                let cookie = MAGIC_COOKIE.to_be_bytes();
                for (index, byte) in octets.iter_mut().enumerate() {
                    *byte ^= cookie[index];
                }
            }
            Some(SocketAddr::from((octets, port)))
        }
        // IPv6 XOR uses the cookie followed by the transaction ID; not
        // requested by this client, so treat it as an unsupported family.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::thread;

    fn xor_attribute(address: SocketAddr) -> Vec<u8> {
        let SocketAddr::V4(address) = address else {
            unreachable!()
        };
        let cookie = MAGIC_COOKIE.to_be_bytes();
        let mut octets = address.ip().octets();
        for (index, byte) in octets.iter_mut().enumerate() {
            *byte ^= cookie[index];
        }
        let port = address.port() ^ (MAGIC_COOKIE >> 16) as u16;
        let mut value = vec![0, 0x01];
        value.extend_from_slice(&port.to_be_bytes());
        value.extend_from_slice(&octets);
        value
    }

    fn mapped_attribute(address: SocketAddr) -> Vec<u8> {
        let SocketAddr::V4(address) = address else {
            unreachable!()
        };
        let mut value = vec![0, 0x01];
        value.extend_from_slice(&address.port().to_be_bytes());
        value.extend_from_slice(&address.ip().octets());
        value
    }

    fn response(transaction: [u8; 12], attributes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        out.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
        out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        out.extend_from_slice(&transaction);
        out.extend_from_slice(attributes);
        out
    }

    fn attribute(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        out.extend_from_slice(value);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// One-shot mock STUN server; returns its address and the requester's
    /// source address (the value it echoes back as the mapped address).
    fn mock_stun(
        build: impl Fn([u8; 12], SocketAddr) -> Vec<u8> + Send + 'static,
    ) -> (String, thread::JoinHandle<SocketAddr>) {
        let server = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let mut request = [0u8; 1500];
            let (size, client) = server.recv_from(&mut request).expect("recv");
            assert!(size >= HEADER_LEN);
            assert_eq!(
                u16::from_be_bytes([request[0], request[1]]),
                BINDING_REQUEST
            );
            assert_eq!(
                u32::from_be_bytes([request[4], request[5], request[6], request[7]]),
                MAGIC_COOKIE
            );
            let mut transaction = [0u8; 12];
            transaction.copy_from_slice(&request[8..20]);
            let reply = build(transaction, client);
            server.send_to(&reply, client).expect("send");
            client
        });
        (address.to_string(), handle)
    }

    #[test]
    fn reads_xor_mapped_address() {
        let (server, handle) = mock_stun(|transaction, client| {
            response(
                transaction,
                &attribute(ATTR_XOR_MAPPED, &xor_attribute(client)),
            )
        });
        let address = public_address(&server, Duration::from_secs(2)).expect("address");
        assert_eq!(address, handle.join().expect("server"));
    }

    #[test]
    fn falls_back_to_mapped_address() {
        let (server, handle) = mock_stun(|transaction, client| {
            response(
                transaction,
                &attribute(ATTR_MAPPED, &mapped_attribute(client)),
            )
        });
        let address = public_address(&server, Duration::from_secs(2)).expect("address");
        assert_eq!(address, handle.join().expect("server"));
    }

    #[test]
    fn rejects_a_different_transaction() {
        let server = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("addr");
        thread::spawn(move || {
            let mut request = [0u8; 1500];
            let (_, client) = server.recv_from(&mut request).expect("recv");
            let reply = response([9; 12], &attribute(ATTR_XOR_MAPPED, &xor_attribute(client)));
            let _ = server.send_to(&reply, client);
        });
        assert_eq!(
            public_address(&address.to_string(), Duration::from_millis(300)),
            Err(StunError::Timeout)
        );
    }

    #[test]
    fn parses_a_known_message() {
        let transaction = [7; 12];
        let address: SocketAddr = "198.51.100.9:4242".parse().expect("address");
        let bytes = response(
            transaction,
            &attribute(ATTR_XOR_MAPPED, &xor_attribute(address)),
        );
        assert_eq!(parse_response(&bytes, transaction), Ok(address));
        assert!(matches!(
            parse_response(&bytes, [8; 12]),
            Err(StunError::Protocol(_))
        ));
    }
}
