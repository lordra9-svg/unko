//! Periodic state-hash exchange for NetPlay desync detection.
//!
//! Every peer hashes its own save state at the same frame and sends the hash
//! with its player tag. A monitor compares each peer's hash with the local one
//! and reports the first mismatch per `(frame, player)` pair. Repeated reports
//! for the same frame are suppressed so the UI can show one alert.

use std::collections::{BTreeMap, BTreeSet};

/// Message magic.
const MAGIC: &[u8; 8] = b"UNKODSY1";
/// Header, frame, player and hash.
pub const MESSAGE_LEN: usize = 8 + 4 + 1 + 8;
/// Frames of hashes retained for out-of-order matches.
const HISTORY: u32 = 256;

/// One peer's state hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateHash {
    /// Frame the state belongs to (after that frame was simulated).
    pub frame: u32,
    /// Zero-based player that produced the hash.
    pub player: u8,
    /// Hash of the serialized save state.
    pub hash: u64,
}

impl StateHash {
    /// Encodes the hash for the transport.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MESSAGE_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.push(self.player);
        out.extend_from_slice(&self.hash.to_le_bytes());
        out
    }

    /// Decodes a hash message, rejecting other protocols and bad lengths.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != MESSAGE_LEN || &bytes[..8] != MAGIC {
            return None;
        }
        Some(Self {
            frame: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            player: bytes[12],
            hash: u64::from_le_bytes(bytes[13..21].try_into().ok()?),
        })
    }
}

/// One detected divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesyncInfo {
    /// Frame both sides hashed.
    pub frame: u32,
    /// Peer whose hash differs.
    pub player: u8,
    /// Local hash.
    pub local: u64,
    /// Peer hash.
    pub remote: u64,
}

impl std::fmt::Display for DesyncInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "desync at frame {} with player {} (local {:016x}, remote {:016x})",
            self.frame, self.player, self.local, self.remote
        )
    }
}

/// Compares local and remote state hashes.
#[derive(Debug, Default)]
pub struct DesyncMonitor {
    hashes: BTreeMap<u32, (Option<u64>, BTreeMap<u8, u64>)>,
    reported: BTreeSet<(u32, u8)>,
    checked: u64,
    mismatches: u64,
    last: Option<DesyncInfo>,
}

impl DesyncMonitor {
    /// Records the local hash for `frame`, returning a mismatch if a peer hash
    /// for that frame is already known.
    pub fn record_local(&mut self, frame: u32, hash: u64) -> Option<DesyncInfo> {
        let entry = self.hashes.entry(frame).or_default();
        entry.0 = Some(hash);
        let remotes: Vec<(u8, u64)> = entry
            .1
            .iter()
            .map(|(player, hash)| (*player, *hash))
            .collect();
        let mut found = None;
        for (player, remote) in remotes {
            if let Some(info) = self.compare(frame, player, hash, remote) {
                found = Some(info);
            }
        }
        self.prune();
        found
    }

    /// Records a peer hash, returning a mismatch if the local hash for that
    /// frame is already known.
    pub fn record_remote(&mut self, hash: StateHash) -> Option<DesyncInfo> {
        let entry = self.hashes.entry(hash.frame).or_default();
        entry.1.insert(hash.player, hash.hash);
        let found = entry
            .0
            .and_then(|local| self.compare(hash.frame, hash.player, local, hash.hash));
        self.prune();
        found
    }

    /// Number of compared hashes.
    #[must_use]
    pub const fn checked(&self) -> u64 {
        self.checked
    }

    /// Number of distinct mismatches.
    #[must_use]
    pub const fn mismatches(&self) -> u64 {
        self.mismatches
    }

    /// Most recent mismatch.
    #[must_use]
    pub const fn last(&self) -> Option<DesyncInfo> {
        self.last
    }

    /// Forgets all history and counters.
    ///
    /// Used after both peers agree on a transferred state, so the old
    /// timeline's hashes cannot be compared against the new one.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn compare(&mut self, frame: u32, player: u8, local: u64, remote: u64) -> Option<DesyncInfo> {
        self.checked = self.checked.saturating_add(1);
        if local == remote || !self.reported.insert((frame, player)) {
            return None;
        }
        self.mismatches = self.mismatches.saturating_add(1);
        let info = DesyncInfo {
            frame,
            player,
            local,
            remote,
        };
        self.last = Some(info);
        Some(info)
    }

    fn prune(&mut self) {
        while let Some((&oldest, _)) = self.hashes.iter().next() {
            if oldest + HISTORY >= self.hashes.keys().next_back().copied().unwrap_or(oldest) {
                break;
            }
            self.hashes.remove(&oldest);
        }
        let floor = self.hashes.keys().next().copied().unwrap_or(0);
        self.reported.retain(|(frame, _)| *frame >= floor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_reject_other_magic() {
        let message = StateHash {
            frame: 42,
            player: 3,
            hash: 0x1234_5678_9abc_def0,
        };
        let bytes = message.encode();
        assert_eq!(bytes.len(), MESSAGE_LEN);
        assert_eq!(StateHash::decode(&bytes), Some(message));
        assert_eq!(StateHash::decode(b"UNKOI2...."), None);
        assert_eq!(StateHash::decode(&bytes[..20]), None);
    }

    #[test]
    fn matching_hashes_do_not_report() {
        let mut monitor = DesyncMonitor::default();
        assert_eq!(monitor.record_local(1, 7), None);
        assert_eq!(
            monitor.record_remote(StateHash {
                frame: 1,
                player: 1,
                hash: 7
            }),
            None
        );
        assert_eq!(monitor.checked(), 1);
        assert_eq!(monitor.mismatches(), 0);
        assert_eq!(monitor.last(), None);
    }

    #[test]
    fn detects_and_suppresses_duplicate_reports() {
        let mut monitor = DesyncMonitor::default();
        monitor.record_local(5, 10);
        let info = monitor
            .record_remote(StateHash {
                frame: 5,
                player: 1,
                hash: 11,
            })
            .expect("mismatch");
        assert_eq!(
            info,
            DesyncInfo {
                frame: 5,
                player: 1,
                local: 10,
                remote: 11
            }
        );
        assert_eq!(monitor.mismatches(), 1);
        // The same peer/frame pair is only reported once.
        assert_eq!(
            monitor.record_remote(StateHash {
                frame: 5,
                player: 1,
                hash: 12
            }),
            None
        );
        assert_eq!(monitor.mismatches(), 1);
    }

    #[test]
    fn compares_when_the_remote_hash_arrives_first() {
        let mut monitor = DesyncMonitor::default();
        assert_eq!(
            monitor.record_remote(StateHash {
                frame: 8,
                player: 1,
                hash: 99
            }),
            None
        );
        let info = monitor.record_local(8, 100).expect("mismatch");
        assert_eq!(info.remote, 99);
        assert_eq!(info.local, 100);
    }

    #[test]
    fn prunes_old_history() {
        let mut monitor = DesyncMonitor::default();
        monitor.record_local(0, 1);
        monitor.record_local(400, 2);
        assert_eq!(
            monitor.record_remote(StateHash {
                frame: 0,
                player: 1,
                hash: 3
            }),
            None,
            "frame 0 is older than the retained window"
        );
    }
}

/// Chunked state-transfer magic.
const STATE_MAGIC: &[u8; 8] = b"UNKOSTAT";
/// Acknowledgement magic.
const ACK_MAGIC: &[u8; 8] = b"UNKOSTAK";
/// Chunk payload bytes (fits every transport's datagram limit).
pub const STATE_CHUNK_BYTES: usize = 1024;
/// Chunk header length.
const CHUNK_HEADER: usize = 8 + 4 + 4 + 8 + 2 + 2;
/// Largest accepted state (16 MiB).
const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;

/// One chunk of a serialized save state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChunk {
    /// Frame the snapshot resumes at.
    pub frame: u32,
    /// Complete state length.
    pub total: u32,
    /// FNV-1a checksum of the complete state.
    pub checksum: u64,
    /// Zero-based chunk index.
    pub index: u16,
    /// Number of chunks in this transfer.
    pub count: u16,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

impl StateChunk {
    /// Splits a state into wire chunks.
    #[must_use]
    pub fn split(frame: u32, state: &[u8]) -> Vec<Self> {
        let checksum = fnv_state(state);
        let count = state.len().div_ceil(STATE_CHUNK_BYTES).max(1) as u16;
        if state.is_empty() {
            return vec![Self {
                frame,
                total: 0,
                checksum,
                index: 0,
                count: 1,
                payload: Vec::new(),
            }];
        }
        state
            .chunks(STATE_CHUNK_BYTES)
            .enumerate()
            .map(|(index, payload)| Self {
                frame,
                total: state.len() as u32,
                checksum,
                index: index as u16,
                count,
                payload: payload.to_vec(),
            })
            .collect()
    }

    /// Encodes one chunk.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CHUNK_HEADER + self.payload.len());
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.extend_from_slice(&self.total.to_le_bytes());
        out.extend_from_slice(&self.checksum.to_le_bytes());
        out.extend_from_slice(&self.index.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decodes one chunk and validates its bounds.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CHUNK_HEADER || &bytes[..8] != STATE_MAGIC {
            return None;
        }
        let frame = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        let total = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
        let checksum = u64::from_le_bytes(bytes[16..24].try_into().ok()?);
        let index = u16::from_le_bytes(bytes[24..26].try_into().ok()?);
        let count = u16::from_le_bytes(bytes[26..28].try_into().ok()?);
        if count == 0 || index >= count || total as usize > MAX_STATE_BYTES {
            return None;
        }
        Some(Self {
            frame,
            total,
            checksum,
            index,
            count,
            payload: bytes[CHUNK_HEADER..].to_vec(),
        })
    }
}

/// Acknowledgement that a peer reassembled the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateAck {
    /// Resumed frame.
    pub frame: u32,
    /// Accepted checksum.
    pub checksum: u64,
    /// Peer that accepted it.
    pub player: u8,
}

impl StateAck {
    /// Encodes the acknowledgement.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(21);
        out.extend_from_slice(ACK_MAGIC);
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.extend_from_slice(&self.checksum.to_le_bytes());
        out.push(self.player);
        out
    }

    /// Decodes an acknowledgement.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 21 || &bytes[..8] != ACK_MAGIC {
            return None;
        }
        Some(Self {
            frame: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            checksum: u64::from_le_bytes(bytes[12..20].try_into().ok()?),
            player: bytes[20],
        })
    }
}

/// Sends a serialized state in chunks and retransmits until acknowledged.
///
/// `next_chunk` walks the transfer once; `restart` begins another pass so a
/// caller can pace batches and recover from datagram loss.
#[derive(Debug)]
pub struct StateSender {
    frame: u32,
    checksum: u64,
    chunks: Vec<Vec<u8>>,
    next: usize,
}

impl StateSender {
    /// Splits `state` into encoded chunks for `frame`.
    #[must_use]
    pub fn new(frame: u32, state: &[u8]) -> Self {
        let chunks: Vec<Vec<u8>> = StateChunk::split(frame, state)
            .iter()
            .map(StateChunk::encode)
            .collect();
        Self {
            frame,
            checksum: fnv_state(state),
            chunks,
            next: 0,
        }
    }

    /// Frame the snapshot resumes at.
    #[must_use]
    pub const fn frame(&self) -> u32 {
        self.frame
    }

    /// Checksum of the complete state.
    #[must_use]
    pub const fn checksum(&self) -> u64 {
        self.checksum
    }

    /// Number of chunks in the transfer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the state produced no chunks (never true for a real state).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Next encoded chunk, or `None` once this pass is complete.
    pub fn next_chunk(&mut self) -> Option<&[u8]> {
        let chunk = self.chunks.get(self.next)?;
        self.next += 1;
        Some(chunk)
    }

    /// Restarts the transfer from the first chunk.
    pub fn restart(&mut self) {
        self.next = 0;
    }

    /// Whether `ack` accepts this transfer.
    #[must_use]
    pub fn acknowledged(&self, ack: StateAck) -> bool {
        ack.frame == self.frame && ack.checksum == self.checksum
    }
}

/// Reassembles a chunked state.
#[derive(Debug, Default)]
pub struct StateReassembler {
    frame: u32,
    checksum: u64,
    total: u32,
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
    active: bool,
}

impl StateReassembler {
    /// Accepts a chunk, returning the complete state once every chunk arrived
    /// and the length/checksum match.
    ///
    /// # Errors
    ///
    /// Returns a description when the completed state fails validation.
    pub fn accept(&mut self, chunk: StateChunk) -> Result<Option<Vec<u8>>, String> {
        if !self.active || self.frame != chunk.frame {
            // A different snapshot (or the first chunk) replaces an unfinished
            // transfer: stale chunks cannot mix into the new state.
            self.frame = chunk.frame;
            self.checksum = chunk.checksum;
            self.total = chunk.total;
            self.chunks = vec![None; usize::from(chunk.count)];
            self.received = 0;
            self.active = true;
        } else if self.checksum != chunk.checksum
            || self.total != chunk.total
            || self.chunks.len() != usize::from(chunk.count)
        {
            if self.received == 0 {
                // The transfer already completed; this is a retransmitted
                // chunk. Ignoring it is safe and avoids a spurious error.
                return Ok(None);
            }
            return Err("chunk does not match the transfer in progress".into());
        }
        let slot = &mut self.chunks[usize::from(chunk.index)];
        if slot.is_none() {
            *slot = Some(chunk.payload);
            self.received += 1;
        }
        if self.received < self.chunks.len() {
            return Ok(None);
        }
        let mut state = Vec::with_capacity(self.total as usize);
        for payload in &self.chunks {
            state.extend_from_slice(payload.as_deref().unwrap_or_default());
        }
        self.chunks.clear();
        self.received = 0;
        self.active = false;
        if state.len() != self.total as usize {
            return Err(format!(
                "state length {} does not match {}",
                state.len(),
                self.total
            ));
        }
        if fnv_state(&state) != self.checksum {
            return Err("state checksum mismatch".into());
        }
        Ok(Some(state))
    }
}

fn fnv_state(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn chunks_reassemble_in_order_duplicates_and_reordered() {
        let state: Vec<u8> = (0..5000u32).map(|value| value as u8).collect();
        let chunks = StateChunk::split(12, &state);
        assert_eq!(chunks.len(), 5);
        let mut reassembler = StateReassembler::default();
        // Out of order, with a duplicate and one chunk omitted until the end.
        for index in [2usize, 0, 2, 4, 1] {
            let decoded = StateChunk::decode(&chunks[index].encode()).expect("decode");
            assert_eq!(reassembler.accept(decoded).expect("accept"), None);
        }
        let last = StateChunk::decode(&chunks[3].encode()).expect("decode");
        assert_eq!(
            reassembler.accept(last).expect("complete"),
            Some(state.clone())
        );
        // A retransmitted chunk after completion is ignored, not an error.
        let duplicate = StateChunk::decode(&chunks[3].encode()).expect("decode");
        assert_eq!(reassembler.accept(duplicate).expect("duplicate"), None);
    }

    #[test]
    fn rejects_a_bad_checksum_and_truncated_state() {
        let state = vec![7u8; 1500];

        // Corrupt payload with a valid length: the checksum must catch it.
        let mut chunks = StateChunk::split(3, &state);
        let last = chunks.len() - 1;
        chunks[last].payload[0] ^= 0x01;
        let mut reassembler = StateReassembler::default();
        let mut error = None;
        for chunk in chunks {
            if let Err(err) = reassembler.accept(chunk) {
                error = Some(err);
            }
        }
        assert!(
            error.as_deref().unwrap_or_default().contains("checksum"),
            "expected a checksum error, got {error:?}"
        );

        // Shortened payload with the original declared length.
        let mut chunks = StateChunk::split(4, &state);
        let last = chunks.len() - 1;
        chunks[last].payload.pop();
        let mut reassembler = StateReassembler::default();
        let mut error = None;
        for chunk in chunks {
            if let Err(err) = reassembler.accept(chunk) {
                error = Some(err);
            }
        }
        assert!(
            error.as_deref().unwrap_or_default().contains("length"),
            "expected a length error, got {error:?}"
        );

        // A chunk whose total/count changed mid-transfer is rejected instead
        // of silently restarting the reassembly.
        let mut chunks = StateChunk::split(5, &state);
        chunks[1].total = 10;
        let mut reassembler = StateReassembler::default();
        let mut error = None;
        for chunk in chunks {
            if let Err(err) = reassembler.accept(chunk) {
                error = Some(err);
            }
        }
        assert!(
            error.as_deref().unwrap_or_default().contains("in progress"),
            "expected an inconsistent-chunk error, got {error:?}"
        );
    }

    #[test]
    fn sender_walks_every_chunk_and_matches_acks() {
        let state = vec![3u8; 3000];
        let mut sender = StateSender::new(7, &state);
        assert_eq!(sender.frame(), 7);
        assert_eq!(sender.len(), 3);
        assert_eq!(sender.checksum(), fnv_state(&state));
        let mut sent = 0;
        while sender.next_chunk().is_some() {
            sent += 1;
        }
        assert_eq!(sent, 3);
        assert!(sender.next_chunk().is_none());
        sender.restart();
        assert!(sender.next_chunk().is_some());
        assert!(sender.acknowledged(StateAck {
            frame: 7,
            checksum: fnv_state(&state),
            player: 1
        }));
        assert!(!sender.acknowledged(StateAck {
            frame: 7,
            checksum: 0,
            player: 1
        }));
    }

    #[test]
    fn ack_round_trips() {
        let ack = StateAck {
            frame: 9,
            checksum: 0xabc,
            player: 2,
        };
        assert_eq!(StateAck::decode(&ack.encode()), Some(ack));
        assert_eq!(StateAck::decode(b"short"), None);
    }

    #[test]
    fn ignores_other_protocols() {
        assert_eq!(StateChunk::decode(b"UNKOI2...."), None);
        assert_eq!(StateChunk::decode(&[0u8; CHUNK_HEADER - 1]), None);
    }
}
