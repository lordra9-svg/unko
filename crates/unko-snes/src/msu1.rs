//! MSU1 media-streaming coprocessor.
//!
//! The MSU1 is an SD2SNES/FXPak enhancement chip, not a period coprocessor: it
//! exposes a data ROM (`msu1.data.rom`) and CD-quality audio tracks
//! (`track-N.pcm`) through eight registers at `$2000-$2007`. Games use it for
//! streamed music, voice and FMV. The register behaviour follows the
//! ISC-licensed ares `sfc/coprocessor/msu1` source.
//!
//! The core has no file system: the host attaches the data ROM and each PCM
//! track through [`crate::Snes::set_msu1_data`] and
//! [`crate::Snes::set_msu1_track`]. Attaching data enables the register window,
//! mirroring ares' `msu1.data.rom`-exists board selection.

use std::collections::VecDeque;

/// MSU1 status: revision in bits 0-2.
pub const MSU1_REVISION: u8 = 0x02;

/// Little-endian magic at the start of a track PCM file (`"MSU1"`).
const TRACK_MAGIC: [u8; 4] = *b"MSU1";

/// Sample rate of MSU1 audio tracks.
pub const MSU1_SAMPLE_RATE: i64 = 44_100;

/// One stereo sample produced by the audio track player.
pub type Msu1Sample = (i16, i16);

/// MSU1 register file and streamer state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Msu1 {
    /// Data ROM attached by the host.
    #[serde(skip)]
    data: Vec<u8>,
    /// PCM tracks attached by the host, indexed by track number.
    #[serde(skip)]
    tracks: Vec<Option<Vec<u8>>>,

    /// Four-byte data seek offset, written least significant byte first.
    data_seek_offset: u32,
    /// Current data read offset.
    data_read_offset: u32,

    /// Byte offset of the next audio sample.
    audio_play_offset: u32,
    /// Byte offset of the track's loop point.
    audio_loop_offset: u32,

    /// Selected audio track.
    audio_track: u16,
    /// Audio volume, 0-255.
    audio_volume: u8,

    /// Track and offset of a paused track, restored on re-selection.
    audio_resume_track: u32,
    audio_resume_offset: u32,

    /// The selected track is missing or malformed.
    audio_error: bool,
    /// The audio player is running.
    audio_play: bool,
    /// The audio player loops at `audio_loop_offset` instead of stopping.
    audio_repeat: bool,
    /// The audio player is busy (always false: playback is instantaneous).
    audio_busy: bool,
    /// The data reader is busy (always false).
    data_busy: bool,

    /// Stereo samples produced since the last drain, at 44.1 kHz.
    #[serde(skip)]
    samples: VecDeque<Msu1Sample>,
}

impl Msu1 {
    /// Create a powered-on MSU1 with no media attached.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the data ROM.
    pub fn set_data(&mut self, data: Vec<u8>) {
        self.data = data;
        self.data_open();
    }

    /// Attach (or replace) a PCM track.
    pub fn set_track(&mut self, track: u16, pcm: Vec<u8>) {
        let index = usize::from(track);
        if self.tracks.len() <= index {
            self.tracks.resize(index + 1, None);
        }
        self.tracks[index] = Some(pcm);
        if self.audio_track == track {
            self.audio_open();
        }
    }

    /// Whether a data ROM is attached.
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// The currently selected track.
    pub const fn track(&self) -> u16 {
        self.audio_track
    }

    /// Whether the selected track is currently playing.
    pub const fn playing(&self) -> bool {
        self.audio_play
    }

    /// Reset the register state, keeping the attached media.
    pub fn power(&mut self) {
        self.data_seek_offset = 0;
        self.data_read_offset = 0;
        self.audio_play_offset = 0;
        self.audio_loop_offset = 0;
        self.audio_track = 0;
        self.audio_volume = 0;
        self.audio_resume_track = u32::MAX;
        self.audio_resume_offset = 0;
        self.audio_error = false;
        self.audio_play = false;
        self.audio_repeat = false;
        self.audio_busy = false;
        self.data_busy = false;
        self.samples.clear();
        self.data_open();
        self.audio_open();
    }

    fn data_open(&mut self) {
        let offset = usize::try_from(self.data_read_offset).unwrap_or(usize::MAX);
        if offset > self.data.len() {
            self.data_read_offset = 0;
        }
    }

    /// Reopen the selected track: validate the header and position the player.
    fn audio_open(&mut self) {
        self.audio_loop_offset = 0;
        let Some(track) = self
            .tracks
            .get(usize::from(self.audio_track))
            .and_then(|t| t.as_deref())
        else {
            self.audio_error = true;
            return;
        };
        if track.len() < 8 || track[0..4] != TRACK_MAGIC {
            self.audio_error = true;
            return;
        }
        let loop_sector = u32::from_le_bytes([track[4], track[5], track[6], track[7]]);
        let loop_offset = 8u64 + u64::from(loop_sector) * 4;
        self.audio_loop_offset = if loop_offset > track.len() as u64 {
            8
        } else {
            loop_offset as u32
        };
        if u64::from(self.audio_play_offset) > track.len() as u64 {
            self.audio_play_offset = 8;
        }
        self.audio_error = false;
    }

    /// Read a register. `address` is masked to the eight-register window.
    pub fn read(&mut self, address: u32, mdr: u8) -> u8 {
        match address & 7 {
            0 => {
                let mut status = MSU1_REVISION;
                if self.audio_error {
                    status |= 0x08;
                }
                if self.audio_play {
                    status |= 0x10;
                }
                if self.audio_repeat {
                    status |= 0x20;
                }
                if self.audio_busy {
                    status |= 0x40;
                }
                if self.data_busy {
                    status |= 0x80;
                }
                status
            }
            1 => {
                if self.data_busy {
                    return 0;
                }
                let index = usize::try_from(self.data_read_offset).unwrap_or(usize::MAX);
                let Some(byte) = self.data.get(index).copied() else {
                    return 0;
                };
                self.data_read_offset = self.data_read_offset.wrapping_add(1);
                byte
            }
            2 => b'S',
            3 => b'-',
            4 => b'M',
            5 => b'S',
            6 => b'U',
            7 => b'1',
            _ => mdr,
        }
    }

    /// Write a register. `address` is masked to the eight-register window.
    pub fn write(&mut self, address: u32, data: u8) {
        match address & 7 {
            0 => self.data_seek_offset = (self.data_seek_offset & !0xFF) | u32::from(data),
            1 => self.data_seek_offset = (self.data_seek_offset & !0xFF00) | (u32::from(data) << 8),
            2 => {
                self.data_seek_offset =
                    (self.data_seek_offset & !0xFF_0000) | (u32::from(data) << 16)
            }
            3 => {
                self.data_seek_offset =
                    (self.data_seek_offset & !0xFF00_0000) | (u32::from(data) << 24);
                self.data_read_offset = self.data_seek_offset;
                self.data_open();
            }
            4 => self.audio_track = (self.audio_track & !0xFF) | u16::from(data),
            5 => {
                self.audio_track = (self.audio_track & !0xFF00) | (u16::from(data) << 8);
                self.audio_play = false;
                self.audio_repeat = false;
                self.audio_play_offset = 8;
                if u32::from(self.audio_track) == self.audio_resume_track {
                    self.audio_play_offset = self.audio_resume_offset;
                    self.audio_resume_track = u32::MAX;
                    self.audio_resume_offset = 0;
                }
                self.audio_open();
            }
            6 => self.audio_volume = data,
            7 => {
                if self.audio_busy || self.audio_error {
                    return;
                }
                self.audio_play = data & 1 != 0;
                self.audio_repeat = data & 2 != 0;
                if !self.audio_play && data & 4 != 0 {
                    self.audio_resume_track = u32::from(self.audio_track);
                    self.audio_resume_offset = self.audio_play_offset;
                }
            }
            _ => {}
        }
    }

    /// Produce one 44.1 kHz stereo sample.
    pub fn tick(&mut self, muted: bool) {
        let mut left = 0i16;
        let mut right = 0i16;
        if self.audio_play {
            let track_len = self
                .tracks
                .get(usize::from(self.audio_track))
                .and_then(|t| t.as_ref())
                .map_or(0, Vec::len);
            if track_len == 0 {
                self.audio_play = false;
            } else if u64::from(self.audio_play_offset) >= track_len as u64 {
                if self.audio_repeat {
                    self.audio_play_offset = self.audio_loop_offset;
                } else {
                    self.audio_play = false;
                    self.audio_play_offset = 8;
                }
            } else {
                let offset = self.audio_play_offset as usize;
                self.audio_play_offset = self.audio_play_offset.wrapping_add(4);
                let track = self.tracks[usize::from(self.audio_track)]
                    .as_ref()
                    .expect("track length checked above");
                if offset + 4 <= track.len() {
                    let raw_left = i16::from_le_bytes([track[offset], track[offset + 1]]);
                    let raw_right = i16::from_le_bytes([track[offset + 2], track[offset + 3]]);
                    if !muted {
                        left = scale(raw_left, self.audio_volume);
                        right = scale(raw_right, self.audio_volume);
                    }
                }
            }
        }
        self.samples.push_back((left, right));
    }

    /// Replace the streamer state from a save state while keeping the media
    /// that the host attached to this instance.
    pub fn restore_state(&mut self, saved: &Self) {
        let data = std::mem::take(&mut self.data);
        let tracks = std::mem::take(&mut self.tracks);
        *self = saved.clone();
        self.data = data;
        self.tracks = tracks;
        self.samples.clear();
        self.audio_open();
    }

    /// Take the samples produced since the last drain.
    pub fn drain_samples(&mut self) -> Vec<Msu1Sample> {
        self.samples.drain(..).collect()
    }
}

/// ares `(f64)sample / 32768.0 * volume / 255.0`; the integer form keeps the
/// pre-resampler value exact at full volume and truncates like the float path.
fn scale(sample: i16, volume: u8) -> i16 {
    ((i32::from(sample) * i32::from(volume)) / 255) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(magic: &[u8; 4], loop_sector: u32, samples: &[(i16, i16)]) -> Vec<u8> {
        let mut pcm = Vec::new();
        pcm.extend_from_slice(magic);
        pcm.extend_from_slice(&loop_sector.to_le_bytes());
        for (left, right) in samples {
            pcm.extend_from_slice(&left.to_le_bytes());
            pcm.extend_from_slice(&right.to_le_bytes());
        }
        pcm
    }

    #[test]
    fn signature_and_revision_are_readable() {
        let mut msu1 = Msu1::new();
        assert_eq!(msu1.read(0, 0) & 0x07, MSU1_REVISION);
        assert_eq!(
            [2, 3, 4, 5, 6, 7].map(|address| msu1.read(address, 0)),
            *b"S-MSU1"
        );
    }

    #[test]
    fn data_seek_and_read_walk_the_rom() {
        let mut msu1 = Msu1::new();
        msu1.set_data(vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(msu1.read(1, 0), 0xAA);
        assert_eq!(msu1.read(1, 0), 0xBB);
        // Seek to 2 and read the tail.
        for (index, byte) in 0x02u32.to_le_bytes().into_iter().enumerate() {
            msu1.write(index as u32, byte);
        }
        assert_eq!(msu1.read(1, 0), 0xCC);
        assert_eq!(msu1.read(1, 0), 0xDD);
        assert_eq!(msu1.read(1, 0), 0x00);
    }

    #[test]
    fn audio_track_plays_and_loops() {
        let mut msu1 = Msu1::new();
        msu1.set_track(1, track(b"MSU1", 0, &[(100, -100), (200, -200)]));
        msu1.write(4, 1);
        msu1.write(5, 0); // select track 1
        msu1.write(6, 255); // full volume
        msu1.write(7, 0x03); // play + repeat
        assert_eq!(msu1.read(0, 0) & 0x38, 0x30);
        msu1.tick(false);
        msu1.tick(false);
        assert_eq!(msu1.drain_samples(), vec![(100, -100), (200, -200)]);
        // The tick that notices the end only reseeks; the next one plays the
        // loop point (byte 8) again.
        msu1.tick(false);
        assert_eq!(msu1.drain_samples(), vec![(0, 0)]);
        msu1.tick(false);
        assert_eq!(msu1.drain_samples(), vec![(100, -100)]);
        assert!(msu1.playing());
    }

    #[test]
    fn missing_track_sets_the_error_flag_and_refuses_to_play() {
        let mut msu1 = Msu1::new();
        msu1.write(4, 9);
        msu1.write(5, 0);
        assert_ne!(msu1.read(0, 0) & 0x08, 0);
        msu1.write(7, 0x01);
        assert!(!msu1.playing());
        msu1.tick(false);
        assert_eq!(msu1.drain_samples(), vec![(0, 0)]);
    }

    #[test]
    fn dsp_mute_silences_the_track() {
        let mut msu1 = Msu1::new();
        msu1.set_track(1, track(b"MSU1", 0, &[(1000, 2000), (1000, 2000)]));
        msu1.write(4, 1);
        msu1.write(5, 0);
        msu1.write(6, 128);
        msu1.write(7, 0x01);
        msu1.tick(true);
        msu1.tick(false);
        assert_eq!(msu1.drain_samples(), vec![(0, 0), (501, 1003)]);
    }

    #[test]
    fn malformed_track_is_rejected() {
        let mut msu1 = Msu1::new();
        msu1.set_track(2, vec![0u8; 16]);
        msu1.write(4, 2);
        msu1.write(5, 0);
        assert_ne!(msu1.read(0, 0) & 0x08, 0);
    }
}
