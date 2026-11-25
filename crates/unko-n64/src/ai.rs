//! N64 audio interface (AI) scaffold: DRAM/len/control/status registers and an
//! instant DMA of 16-bit stereo sample pairs from RDRAM into a FIFO.

/// AI register block base.
pub const BASE: u32 = 0x0450_0000;

/// One AI.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    /// `AI_DRAM_ADDR`.
    pub dram_addr: u32,
    /// `AI_LEN` (bytes).
    pub len: u32,
    /// `AI_CONTROL`; bit 0 enables DMA.
    pub control: u32,
    /// `AI_STATUS`.
    pub status: u32,
    /// `AI_DACRATE`.
    pub dacrate: u32,
    /// `AI_BITRATE`.
    pub bitrate: u32,
    fifo: std::collections::VecDeque<(i16, i16)>,
}

impl Ai {
    /// Creates an idle AI.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Audio sample rate implied by `AI_DACRATE`.
    ///
    /// The AI DAC clock is the 48.681818 MHz video clock.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        let divisor = u64::from(self.dacrate) + 1;
        let rate = 48_681_818u64 / divisor;
        if rate == 0 {
            48_000.0
        } else {
            rate as f32
        }
    }

    /// Reads an AI register (`offset` from `0x0450_0000`).
    #[must_use]
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset & 0x1F {
            0x00 => self.dram_addr,
            0x04 => self.len,
            0x08 => self.control,
            0x0C => self.status,
            0x10 => self.dacrate,
            0x14 => self.bitrate,
            _ => 0,
        }
    }

    /// Writes an AI register; `AI_LEN` starts a DMA when control is enabled.
    pub fn write_register(&mut self, offset: u32, value: u32, rdram: &[u8]) {
        match offset & 0x1F {
            0x00 => self.dram_addr = value & 0x00FF_FFFF,
            0x04 => {
                self.len = value & 0x00FF_FFFF;
                self.dma(rdram);
            }
            0x08 => self.control = value,
            0x0C => self.status = value & !0x8000_0000,
            0x10 => self.dacrate = value & 0x3FFF,
            0x14 => self.bitrate = value & 0x7F,
            _ => {}
        }
    }

    fn dma(&mut self, rdram: &[u8]) {
        if self.control & 1 == 0 {
            return;
        }
        let address = self.dram_addr as usize;
        let samples = self.len as usize / 4;
        for index in 0..samples {
            let offset = address + index * 4;
            let Some(bytes) = rdram.get(offset..offset + 4) else {
                break;
            };
            let left = i16::from_be_bytes([bytes[0], bytes[1]]);
            let right = i16::from_be_bytes([bytes[2], bytes[3]]);
            self.fifo.push_back((left, right));
            if self.fifo.len() > 1 << 20 {
                self.fifo.pop_front();
            }
        }
        self.status &= !0x8000_0000;
    }

    /// Drains the FIFO.
    pub fn take_samples(&mut self) -> Vec<(i16, i16)> {
        self.fifo.drain(..).collect()
    }
}
