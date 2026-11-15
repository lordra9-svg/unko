//! Encoded per-frame input messages.
//!
//! Netplay is peer-to-peer: every instance owns one controller port and sends
//! only that port's value, tagged with its player index. Receivers merge the
//! tags into the four-port frame the session simulates.

use unko_core::InputState;

/// Message magic and version (`UNKOI2` adds the player tag).
const MAGIC: &[u8; 6] = b"UNKOI2";
/// Header, frame, player tag and one port value.
pub const INPUT_MESSAGE_LEN: usize = 6 + 4 + 1 + 2;

/// One frame's input from one player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFrame {
    /// Frame number this input applies to.
    pub frame: u32,
    /// Zero-based controller port owned by the sender.
    pub player: u8,
    /// Raw [`InputState`] bits for the sender's port.
    pub input: u16,
}

impl InputFrame {
    /// Builds a frame from a core input state.
    #[must_use]
    pub fn new(frame: u32, player: u8, input: InputState) -> Self {
        Self {
            frame,
            player,
            input: input.bits(),
        }
    }

    /// Encodes the frame for [`crate::Transport`].
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(INPUT_MESSAGE_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.push(self.player);
        out.extend_from_slice(&self.input.to_le_bytes());
        out
    }

    /// Decodes a frame, rejecting a wrong magic, length, player tag or
    /// trailing bytes.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != INPUT_MESSAGE_LEN || &bytes[..6] != MAGIC {
            return None;
        }
        let frame = u32::from_le_bytes(bytes[6..10].try_into().ok()?);
        let player = bytes[10];
        if player > 3 {
            return None;
        }
        let input = u16::from_le_bytes(bytes[11..13].try_into().ok()?);
        Some(Self {
            frame,
            player,
            input,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_player_slot() {
        for player in 0..=3u8 {
            let frame = InputFrame {
                frame: 0x1234_5678,
                player,
                input: 0x0203,
            };
            let bytes = frame.encode();
            assert_eq!(bytes.len(), INPUT_MESSAGE_LEN);
            assert_eq!(InputFrame::decode(&bytes), Some(frame));
        }
    }

    #[test]
    fn rejects_malformed_messages() {
        let frame = InputFrame {
            frame: 1,
            player: 2,
            input: 0,
        };
        let mut bytes = frame.encode();
        assert!(InputFrame::decode(&bytes[..bytes.len() - 1]).is_none());
        bytes[0] = b'X';
        assert!(InputFrame::decode(&bytes).is_none());
        let mut extended = frame.encode();
        extended.push(0);
        assert!(InputFrame::decode(&extended).is_none());
        let mut bad_player = frame.encode();
        bad_player[10] = 4;
        assert!(InputFrame::decode(&bad_player).is_none());
    }

    #[test]
    fn converts_a_core_state() {
        let frame = InputFrame::new(9, 1, InputState::from_bits(1));
        assert_eq!(frame.input, 1);
        assert_eq!(InputFrame::decode(&frame.encode()), Some(frame));
    }
}
