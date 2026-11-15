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
