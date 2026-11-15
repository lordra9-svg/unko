//! Serialized-state ring used by rollback resimulation.

use std::collections::VecDeque;

/// A state captured before `frame` plus that frame's four-port input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFrame {
    /// Frame that follows the captured state.
    pub frame: u32,
    /// Raw input bits for the four ports.
    pub inputs: [u16; 4],
    /// Core save state taken immediately before `frame`.
    pub state: Vec<u8>,
}

/// Bounded ring of recent states, oldest first.
#[derive(Debug, Clone, Default)]
pub struct RollbackBuffer {
    capacity: usize,
    frames: VecDeque<StoredFrame>,
}

impl RollbackBuffer {
    /// Creates a buffer holding at most `capacity` frames.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: VecDeque::new(),
        }
    }

    /// Configured capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of stored frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether no frame is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Oldest stored frame number.
    #[must_use]
    pub fn oldest_frame(&self) -> Option<u32> {
        self.frames.front().map(|frame| frame.frame)
    }

    /// Newest stored frame number.
    #[must_use]
    pub fn newest_frame(&self) -> Option<u32> {
        self.frames.back().map(|frame| frame.frame)
    }

    /// Stores one frame, dropping the oldest entry at capacity.
    ///
    /// Consecutive frames must be pushed in order; a duplicate or older frame
    /// is ignored so a retransmit cannot corrupt the ring.
    pub fn push(&mut self, frame: StoredFrame) {
        if let Some(newest) = self.newest_frame() {
            if frame.frame <= newest {
                return;
            }
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// Iterates stored frames, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &StoredFrame> {
        self.frames.iter()
    }

    /// Finds a stored frame.
    #[must_use]
    pub fn find(&self, frame: u32) -> Option<&StoredFrame> {
        self.frames.iter().find(|entry| entry.frame == frame)
    }

    /// Discards `frame` and everything newer, keeping older states available
    /// for a resimulation.
    pub fn truncate_from(&mut self, frame: u32) {
        while self.frames.back().is_some_and(|entry| entry.frame >= frame) {
            self.frames.pop_back();
        }
    }

    /// Drops all entries.
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(number: u32) -> StoredFrame {
        StoredFrame {
            frame: number,
            inputs: [u16::from(number as u8); 4],
            state: vec![number as u8; 4],
        }
    }

    #[test]
    fn evicts_the_oldest_at_capacity() {
        let mut buffer = RollbackBuffer::new(3);
        for number in 0..5 {
            buffer.push(frame(number));
        }
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.oldest_frame(), Some(2));
        assert_eq!(buffer.newest_frame(), Some(4));
        assert!(buffer.find(0).is_none());
        assert!(buffer.find(2).is_some());
    }

    #[test]
    fn ignores_duplicates_and_older_frames() {
        let mut buffer = RollbackBuffer::new(4);
        buffer.push(frame(2));
        buffer.push(frame(2));
        buffer.push(frame(1));
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.newest_frame(), Some(2));
    }

    #[test]
    fn truncate_keeps_older_states_and_drops_the_rest() {
        let mut buffer = RollbackBuffer::new(6);
        for number in 0..5 {
            buffer.push(frame(number));
        }
        buffer.truncate_from(2);
        assert_eq!(buffer.newest_frame(), Some(1));
        assert_eq!(buffer.len(), 2);
        buffer.truncate_from(0);
        assert!(buffer.is_empty());
    }
}
