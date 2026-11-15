//! Encoded per-frame input messages.

use unko_core::InputState;

/// Message magic and version.
const MAGIC: &[u8; 6] = b"UNKOI1";
/// Header plus frame plus four ports.
pub const INPUT_MESSAGE_LEN: usize = 6 + 4 + 4 * 2;

/// One frame's four-port input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFrame {
    /// Frame number this input applies to.
    pub frame: u32,
    /// Raw [`InputState`] bits for ports 1-4.
    pub ports: [u16; 4],
}

impl InputFrame {
    /// Builds a frame from the core's input states.
    #[must_use]
    pub fn from_states(frame: u32, states: [InputState; 4]) -> Self {
        Self {
            frame,
            ports: states.map(InputState::bits),
        }
    }

    /// Expands the frame into core input states.
    #[must_use]
    pub fn to_states(self) -> [InputState; 4] {
        self.ports.map(InputState::from_bits)
    }

    /// Encodes the frame for [`crate::Transport`].
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(INPUT_MESSAGE_LEN);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.frame.to_le_bytes());
        for port in self.ports {
            out.extend_from_slice(&port.to_le_bytes());
        }
        out
    }

    /// Decodes a frame, rejecting a wrong magic, length or trailing bytes.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != INPUT_MESSAGE_LEN || &bytes[..6] != MAGIC {
            return None;
        }
        let frame = u32::from_le_bytes(bytes[6..10].try_into().ok()?);
        let mut ports = [0u16; 4];
        for (index, port) in ports.iter_mut().enumerate() {
            let start = 10 + index * 2;
            *port = u16::from_le_bytes(bytes[start..start + 2].try_into().ok()?);
        }
        Some(Self { frame, ports })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_ports() {
        let frame = InputFrame {
            frame: 0x1234_5678,
            ports: [0x0001, 0x0203, 0x0405, 0xFFFF],
        };
        let bytes = frame.encode();
        assert_eq!(bytes.len(), INPUT_MESSAGE_LEN);
        assert_eq!(InputFrame::decode(&bytes), Some(frame));
    }

    #[test]
    fn rejects_malformed_messages() {
        let frame = InputFrame {
            frame: 1,
            ports: [0; 4],
        };
        let mut bytes = frame.encode();
        assert!(InputFrame::decode(&bytes[..bytes.len() - 1]).is_none());
        bytes[0] = b'X';
        assert!(InputFrame::decode(&bytes).is_none());
        let mut extended = frame.encode();
        extended.push(0);
        assert!(InputFrame::decode(&extended).is_none());
    }

    #[test]
    fn converts_between_states_and_bits() {
        let states = [
            InputState::from_bits(InputState::A),
            InputState::from_bits(InputState::START | InputState::SELECT),
            InputState::empty(),
            InputState::from_bits(InputState::B),
        ];
        let frame = InputFrame::from_states(9, states);
        assert_eq!(frame.to_states(), states);
    }
}
