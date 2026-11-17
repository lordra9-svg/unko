//! S-DD1 decompression coprocessor.
//!
//! Ported from the ares `SDD1` component. The chip sits between the S-CPU and
//! the Game Pak ROM: it exposes two enable registers and four MMC bank
//! registers at `$4800-$480F`, and it can stream-decompress graphics directly
//! into a DMA transfer. Everything else is a straight ROM window.
//!
//! The decompressor is the `q_zip`/Golomb-Rice coder from ares: an input
//! manager, eight bit generators, a probability-estimation module, a context
//! model and the output logic. It is entropy coded, so any deviation shows up
//! immediately as garbage pixels.

/// One captured S-CPU DMA channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Sdd1Dma {
    /// `$43x2-$43x4`: A-bus source address (24 bits).
    pub address: u32,
    /// `$43x5-$43x6`: transfer size in bytes.
    pub size: u16,
}

/// The S-DD1's MMC bank registers, as seen by the decompressor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Sdd1Banks {
    /// `$4804`: banks `$C0-$CF`.
    pub bank0: u8,
    /// `$4805`: banks `$D0-$DF`.
    pub bank1: u8,
    /// `$4806`: banks `$E0-$EF`.
    pub bank2: u8,
    /// `$4807`: banks `$F0-$FF`.
    pub bank3: u8,
}

impl Sdd1Banks {
    /// The 4-bit bank number selected for a 24-bit address (`address >> 20`).
    pub const fn select(&self, address: u32) -> u8 {
        match (address >> 20) & 3 {
            0 => self.bank0,
            1 => self.bank1,
            2 => self.bank2,
            _ => self.bank3,
        }
    }
}

/// One entry of the probability-estimation evolution table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Evolution {
    code_number: u8,
    next_if_mps: u8,
    next_if_lps: u8,
}

const EVOLUTION: [Evolution; 33] = [
    Evolution {
        code_number: 0,
        next_if_mps: 25,
        next_if_lps: 25,
    },
    Evolution {
        code_number: 0,
        next_if_mps: 2,
        next_if_lps: 1,
    },
    Evolution {
        code_number: 0,
        next_if_mps: 3,
        next_if_lps: 1,
    },
    Evolution {
        code_number: 0,
        next_if_mps: 4,
        next_if_lps: 2,
    },
    Evolution {
        code_number: 0,
        next_if_mps: 5,
        next_if_lps: 3,
    },
    Evolution {
        code_number: 1,
        next_if_mps: 6,
        next_if_lps: 4,
    },
    Evolution {
        code_number: 1,
        next_if_mps: 7,
        next_if_lps: 5,
    },
    Evolution {
        code_number: 1,
        next_if_mps: 8,
        next_if_lps: 6,
    },
    Evolution {
        code_number: 1,
        next_if_mps: 9,
        next_if_lps: 7,
    },
    Evolution {
        code_number: 2,
        next_if_mps: 10,
        next_if_lps: 8,
    },
    Evolution {
        code_number: 2,
        next_if_mps: 11,
        next_if_lps: 9,
    },
    Evolution {
        code_number: 2,
        next_if_mps: 12,
        next_if_lps: 10,
    },
    Evolution {
        code_number: 2,
        next_if_mps: 13,
        next_if_lps: 11,
    },
    Evolution {
        code_number: 3,
        next_if_mps: 14,
        next_if_lps: 12,
    },
    Evolution {
        code_number: 3,
        next_if_mps: 15,
        next_if_lps: 13,
    },
    Evolution {
        code_number: 3,
        next_if_mps: 16,
        next_if_lps: 14,
    },
    Evolution {
        code_number: 3,
        next_if_mps: 17,
        next_if_lps: 15,
    },
    Evolution {
        code_number: 4,
        next_if_mps: 18,
        next_if_lps: 16,
    },
    Evolution {
        code_number: 4,
        next_if_mps: 19,
        next_if_lps: 17,
    },
    Evolution {
        code_number: 5,
        next_if_mps: 20,
        next_if_lps: 18,
    },
    Evolution {
        code_number: 5,
        next_if_mps: 21,
        next_if_lps: 19,
    },
    Evolution {
        code_number: 6,
        next_if_mps: 22,
        next_if_lps: 20,
    },
    Evolution {
        code_number: 6,
        next_if_mps: 23,
        next_if_lps: 21,
    },
    Evolution {
        code_number: 7,
        next_if_mps: 24,
        next_if_lps: 22,
    },
    Evolution {
        code_number: 7,
        next_if_mps: 24,
        next_if_lps: 23,
    },
    Evolution {
        code_number: 0,
        next_if_mps: 26,
        next_if_lps: 1,
    },
    Evolution {
        code_number: 1,
        next_if_mps: 27,
        next_if_lps: 2,
    },
    Evolution {
        code_number: 2,
        next_if_mps: 28,
        next_if_lps: 4,
    },
    Evolution {
        code_number: 3,
        next_if_mps: 29,
        next_if_lps: 8,
    },
    Evolution {
        code_number: 4,
        next_if_mps: 30,
        next_if_lps: 12,
    },
    Evolution {
        code_number: 5,
        next_if_mps: 31,
        next_if_lps: 16,
    },
    Evolution {
        code_number: 6,
        next_if_mps: 32,
        next_if_lps: 18,
    },
    Evolution {
        code_number: 7,
        next_if_mps: 24,
        next_if_lps: 22,
    },
];

/// Golomb-code run counts (ares `GCD::runCount`).
const RUN_COUNT: [u8; 256] = [
    0x00, 0x00, 0x01, 0x00, 0x03, 0x01, 0x02, 0x00, 0x07, 0x03, 0x05, 0x01, 0x06, 0x02, 0x04, 0x00,
    0x0f, 0x07, 0x0b, 0x03, 0x0d, 0x05, 0x09, 0x01, 0x0e, 0x06, 0x0a, 0x02, 0x0c, 0x04, 0x08, 0x00,
    0x1f, 0x0f, 0x17, 0x07, 0x1b, 0x0b, 0x13, 0x03, 0x1d, 0x0d, 0x15, 0x05, 0x19, 0x09, 0x11, 0x01,
    0x1e, 0x0e, 0x16, 0x06, 0x1a, 0x0a, 0x12, 0x02, 0x1c, 0x0c, 0x14, 0x04, 0x18, 0x08, 0x10, 0x00,
    0x3f, 0x1f, 0x2f, 0x0f, 0x37, 0x17, 0x27, 0x07, 0x3b, 0x1b, 0x2b, 0x0b, 0x33, 0x13, 0x23, 0x03,
    0x3d, 0x1d, 0x2d, 0x0d, 0x35, 0x15, 0x25, 0x05, 0x39, 0x19, 0x29, 0x09, 0x31, 0x11, 0x21, 0x01,
    0x3e, 0x1e, 0x2e, 0x0e, 0x36, 0x16, 0x26, 0x06, 0x3a, 0x1a, 0x2a, 0x0a, 0x32, 0x12, 0x22, 0x02,
    0x3c, 0x1c, 0x2c, 0x0c, 0x34, 0x14, 0x24, 0x04, 0x38, 0x18, 0x28, 0x08, 0x30, 0x10, 0x20, 0x00,
    0x7f, 0x3f, 0x5f, 0x1f, 0x6f, 0x2f, 0x4f, 0x0f, 0x77, 0x37, 0x57, 0x17, 0x67, 0x27, 0x47, 0x07,
    0x7b, 0x3b, 0x5b, 0x1b, 0x6b, 0x2b, 0x4b, 0x0b, 0x73, 0x33, 0x53, 0x13, 0x63, 0x23, 0x43, 0x03,
    0x7d, 0x3d, 0x5d, 0x1d, 0x6d, 0x2d, 0x4d, 0x0d, 0x75, 0x35, 0x55, 0x15, 0x65, 0x25, 0x45, 0x05,
    0x79, 0x39, 0x59, 0x19, 0x69, 0x29, 0x49, 0x09, 0x71, 0x31, 0x51, 0x11, 0x61, 0x21, 0x41, 0x01,
    0x7e, 0x3e, 0x5e, 0x1e, 0x6e, 0x2e, 0x4e, 0x0e, 0x76, 0x36, 0x56, 0x16, 0x66, 0x26, 0x46, 0x06,
    0x7a, 0x3a, 0x5a, 0x1a, 0x6a, 0x2a, 0x4a, 0x0a, 0x72, 0x32, 0x52, 0x12, 0x62, 0x22, 0x42, 0x02,
    0x7c, 0x3c, 0x5c, 0x1c, 0x6c, 0x2c, 0x4c, 0x0c, 0x74, 0x34, 0x54, 0x14, 0x64, 0x24, 0x44, 0x04,
    0x78, 0x38, 0x58, 0x18, 0x68, 0x28, 0x48, 0x08, 0x70, 0x30, 0x50, 0x10, 0x60, 0x20, 0x40, 0x00,
];

/// The S-DD1 entropy decompressor.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Sdd1Decompressor {
    /// Input manager: byte offset into the MMC-mapped ROM.
    offset: u32,
    /// Input manager: bits consumed in the current byte.
    bit_count: u32,
    /// Bits generator: remaining MPS run length per code number.
    bg_mps_count: [u8; 8],
    /// Bits generator: pending LPS symbol per code number.
    bg_lps_index: [bool; 8],
    /// Probability estimation: per-context evolution status.
    context_status: [u8; 32],
    /// Probability estimation: per-context most-probable symbol.
    context_mps: [u8; 32],
    /// Context model: `$C0` bits of the stream header.
    bitplanes_info: u8,
    /// Context model: `$30` bits of the stream header.
    context_bits_info: u8,
    /// Context model: bits decoded so far (8-bit, wraps).
    bit_number: u8,
    /// Context model: the bitplane being decoded.
    current_bitplane: u8,
    /// Context model: the previous bits of each bitplane.
    previous_bitplane_bits: [u16; 8],
    /// Output logic: the `$C0` bits of the stream header.
    ol_bitplanes_info: u8,
    /// Output logic: bit mask / first / second plane.
    r0: u8,
    /// Output logic: first plane.
    r1: u8,
    /// Output logic: second plane.
    r2: u8,
}

/// Read one byte of the MMC-mapped ROM.
///
/// ares reads the `ReadableMemory` directly; an out-of-range bank register
/// would be out of bounds there, so this returns 0 instead of panicking.
fn mmc_read(rom: &[u8], banks: &Sdd1Banks, address: u32) -> u8 {
    let bank = u32::from(banks.select(address) & 0x0F);
    let offset = (bank << 20) | (address & 0xFFFFF);
    rom.get(offset as usize).copied().unwrap_or(0)
}

impl Sdd1Decompressor {
    /// Reset and load the stream header at `offset` (ares `Decompressor::init`).
    fn init(&mut self, offset: u32, rom: &[u8], banks: &Sdd1Banks) {
        self.offset = offset;
        self.bit_count = 4;
        self.bg_mps_count = [0; 8];
        self.bg_lps_index = [false; 8];
        self.context_status = [0; 32];
        self.context_mps = [0; 32];
        self.bit_number = 0;
        self.previous_bitplane_bits = [0; 8];

        let header = mmc_read(rom, banks, offset);
        self.bitplanes_info = header & 0xC0;
        self.context_bits_info = header & 0x30;
        self.current_bitplane = match self.bitplanes_info {
            0x00 => 1,
            0x40 => 7,
            0x80 => 3,
            _ => 0,
        };
        self.ol_bitplanes_info = header & 0xC0;
        self.r0 = 0x01;
    }

    /// Fetch one Golomb-Rice code word (ares `IM::getCodeWord`).
    fn get_code_word(&mut self, code_length: u8, rom: &[u8], banks: &Sdd1Banks) -> u8 {
        let first = mmc_read(rom, banks, self.offset);
        let mut code_word = ((u32::from(first) << self.bit_count) & 0xFF) as u8;
        self.bit_count += 1;

        if code_word & 0x80 != 0 {
            let second = mmc_read(rom, banks, self.offset.wrapping_add(1));
            code_word |= ((u32::from(second) >> (9 - self.bit_count)) & 0xFF) as u8;
            self.bit_count += u32::from(code_length);
        }

        if self.bit_count & 0x08 != 0 {
            self.offset = self.offset.wrapping_add(1);
            self.bit_count &= 0x07;
        }
        code_word
    }

    /// Decode one run (ares `GCD::getRunCount`).
    fn get_run_count(
        &mut self,
        code_number: u8,
        rom: &[u8],
        banks: &Sdd1Banks,
        mps_count: &mut u8,
        lps_index: &mut bool,
    ) {
        let code_word = self.get_code_word(code_number, rom, banks);
        if code_word & 0x80 != 0 {
            *lps_index = true;
            *mps_count = RUN_COUNT[usize::from(code_word >> (code_number ^ 0x07))];
        } else {
            *mps_count = 1u8 << code_number;
        }
    }

    /// Decode one bit from a bits generator (ares `BG::getBit`).
    fn bg_get_bit(
        &mut self,
        code_number: u8,
        rom: &[u8],
        banks: &Sdd1Banks,
        end_of_run: &mut bool,
    ) -> u8 {
        let index = usize::from(code_number & 7);
        if !(self.bg_mps_count[index] != 0 || self.bg_lps_index[index]) {
            let mut mps = self.bg_mps_count[index];
            let mut lps = self.bg_lps_index[index];
            self.get_run_count(code_number, rom, banks, &mut mps, &mut lps);
            self.bg_mps_count[index] = mps;
            self.bg_lps_index[index] = lps;
        }

        let bit;
        if self.bg_mps_count[index] != 0 {
            bit = 0;
            self.bg_mps_count[index] -= 1;
        } else {
            bit = 1;
            self.bg_lps_index[index] = false;
        }

        *end_of_run = !(self.bg_mps_count[index] != 0 || self.bg_lps_index[index]);
        bit
    }

    /// Decode one bit through the probability-estimation module
    /// (ares `PEM::getBit`).
    fn pem_get_bit(&mut self, context: u8, rom: &[u8], banks: &Sdd1Banks) -> u8 {
        let index = usize::from(context & 31);
        // The evolution table has 33 states (0..=32): `next_if_mps` for state
        // 31 is 32, so the status must *not* be truncated to 5 bits here.
        let current_status = self.context_status[index].min(32);
        let current_mps = self.context_mps[index] & 1;
        let evolution = EVOLUTION[usize::from(current_status)];

        let mut end_of_run = false;
        let bit = self.bg_get_bit(evolution.code_number, rom, banks, &mut end_of_run);

        if end_of_run {
            if bit != 0 {
                if current_status & 0xFE == 0 {
                    self.context_mps[index] ^= 0x01;
                }
                self.context_status[index] = evolution.next_if_lps;
            } else {
                self.context_status[index] = evolution.next_if_mps;
            }
        }

        bit ^ current_mps
    }

    /// Decode one bit through the context model (ares `CM::getBit`).
    fn cm_get_bit(&mut self, rom: &[u8], banks: &Sdd1Banks) -> u8 {
        match self.bitplanes_info {
            0x00 => self.current_bitplane ^= 0x01,
            0x40 => {
                self.current_bitplane ^= 0x01;
                if self.bit_number & 0x7F == 0 {
                    self.current_bitplane = (self.current_bitplane.wrapping_add(2)) & 0x07;
                }
            }
            0x80 => {
                self.current_bitplane ^= 0x01;
                if self.bit_number & 0x7F == 0 {
                    self.current_bitplane ^= 0x02;
                }
            }
            _ => self.current_bitplane = self.bit_number & 0x07,
        }

        let plane = usize::from(self.current_bitplane & 0x07);
        let context_bits = self.previous_bitplane_bits[plane];
        let mut current_context = (self.current_bitplane & 0x01) << 4;
        current_context |= match self.context_bits_info {
            0x00 => (((context_bits & 0x01C0) >> 5) as u8) | ((context_bits & 0x0001) as u8),
            0x10 => (((context_bits & 0x0180) >> 5) as u8) | ((context_bits & 0x0001) as u8),
            0x20 => (((context_bits & 0x00C0) >> 5) as u8) | ((context_bits & 0x0001) as u8),
            _ => (((context_bits & 0x0180) >> 5) as u8) | ((context_bits & 0x0003) as u8),
        };

        let bit = self.pem_get_bit(current_context, rom, banks);
        let slot = &mut self.previous_bitplane_bits[plane];
        *slot = (*slot << 1) | u16::from(bit);
        self.bit_number = self.bit_number.wrapping_add(1);
        bit
    }

    /// Produce the next decompressed byte (ares `OL::decompress`).
    fn ol_decompress(&mut self, rom: &[u8], banks: &Sdd1Banks) -> u8 {
        match self.ol_bitplanes_info {
            0x00 | 0x40 | 0x80 => {
                if self.r0 == 0 {
                    self.r0 = !self.r0;
                    return self.r2;
                }
                self.r0 = 0x80;
                self.r1 = 0;
                self.r2 = 0;
                while self.r0 != 0 {
                    if self.cm_get_bit(rom, banks) != 0 {
                        self.r1 |= self.r0;
                    }
                    if self.cm_get_bit(rom, banks) != 0 {
                        self.r2 |= self.r0;
                    }
                    self.r0 >>= 1;
                }
                self.r1
            }
            _ => {
                self.r0 = 0x01;
                self.r1 = 0;
                while self.r0 != 0 {
                    if self.cm_get_bit(rom, banks) != 0 {
                        self.r1 |= self.r0;
                    }
                    self.r0 <<= 1;
                }
                self.r1
            }
        }
    }

    /// Decode one byte.
    pub fn read(&mut self, rom: &[u8], banks: &Sdd1Banks) -> u8 {
        self.ol_decompress(rom, banks)
    }
}

/// The S-DD1 coprocessor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sdd1 {
    /// `$4800`: hard enable bitmask (one bit per DMA channel).
    pub r4800: u8,
    /// `$4801`: soft enable bitmask; cleared per channel when its DMA ends.
    pub r4801: u8,
    /// `$4804-$4807`: MMC bank registers.
    pub banks: Sdd1Banks,
    /// Shadow copy of the eight DMA channels' source and size registers.
    pub dma: [Sdd1Dma; 8],
    /// A stream decompression is in progress.
    pub dma_ready: bool,
    /// The entropy decompressor.
    pub decompressor: Sdd1Decompressor,
    /// Decompressed bytes emitted (debug).
    pub bytes: u64,
    /// Streams started (debug).
    pub streams: u64,
}

impl Default for Sdd1 {
    fn default() -> Self {
        Self {
            r4800: 0,
            r4801: 0,
            banks: Sdd1Banks {
                bank0: 0,
                bank1: 1,
                bank2: 2,
                bank3: 3,
            },
            dma: [Sdd1Dma::default(); 8],
            dma_ready: false,
            decompressor: Sdd1Decompressor::default(),
            bytes: 0,
            streams: 0,
        }
    }
}

impl Sdd1 {
    /// Create a powered-on S-DD1 (ares `SDD1::power`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to the power-on state (ares `SDD1::power`).
    pub fn power(&mut self) {
        *self = Self::default();
    }

    /// Whether an S-CPU address hits the register window
    /// (`$00-$3F,$80-$BF:$4800-$480F`).
    pub const fn in_io_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        let offset = address & 0xFFFF;
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && (offset >= 0x4800 && offset <= 0x480F)
    }

    /// Whether an S-CPU address hits the MCU window
    /// (`$00-$3F,$80-$BF:$8000-$FFFF` or `$C0-$FF:$0000-$FFFF`).
    pub const fn in_mcu_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        let offset = address & 0xFFFF;
        if bank >= 0xC0 {
            return true;
        }
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x8000
    }

    /// Read a register byte (ares `SDD1::ioRead`).
    ///
    /// `$4802-$4803` and `$4808-$480F` fall through to the raw ROM at that
    /// 24-bit offset, exactly like ares.
    pub fn read_io(&self, address: u32, rom: &[u8]) -> u8 {
        match 0x4800 | (address & 0x0F) {
            0x4800 => self.r4800,
            0x4801 => self.r4801,
            0x4804 => self.banks.bank0,
            0x4805 => self.banks.bank1,
            0x4806 => self.banks.bank2,
            0x4807 => self.banks.bank3,
            other => rom.get(other as usize).copied().unwrap_or(0),
        }
    }

    /// Write a register byte (ares `SDD1::ioWrite`).
    pub fn write_io(&mut self, address: u32, data: u8) {
        match 0x4800 | (address & 0x0F) {
            0x4800 => self.r4800 = data,
            0x4801 => self.r4801 = data,
            0x4804 => self.banks.bank0 = data & 0x8F,
            0x4805 => self.banks.bank1 = data & 0x8F,
            0x4806 => self.banks.bank2 = data & 0x8F,
            0x4807 => self.banks.bank3 = data & 0x8F,
            _ => {}
        }
    }

    /// Observe an S-CPU DMA register write (ares' `$4300-$437F` hook).
    ///
    /// The S-DD1 only needs the A-bus source address and the transfer size; it
    /// uses them to recognise the decompressing DMA inside [`Self::mcu_read`].
    pub fn note_dma_write(&mut self, offset: u16, data: u8) {
        let channel = usize::from((offset >> 4) & 0x07);
        let dma = &mut self.dma[channel];
        match offset & 0x0F {
            0x2 => dma.address = (dma.address & 0xFFFF00) | u32::from(data),
            0x3 => dma.address = (dma.address & 0xFF00FF) | (u32::from(data) << 8),
            0x4 => dma.address = (dma.address & 0x00FFFF) | (u32::from(data) << 16),
            0x5 => dma.size = (dma.size & 0xFF00) | u16::from(data),
            0x6 => dma.size = (dma.size & 0x00FF) | (u16::from(data) << 8),
            _ => {}
        }
    }

    /// Read the ROM through the MMC bank registers (ares `SDD1::mmcRead`).
    pub fn mmc_read(&self, address: u32, rom: &[u8]) -> u8 {
        mmc_read(rom, &self.banks, address)
    }

    /// Read the MCU window (ares `SDD1::mcuRead`).
    pub fn mcu_read(&mut self, address: u32, rom: &[u8]) -> u8 {
        // $00-$3F,$80-$BF:$8000-$FFFF: LoROM conversion, with the two
        // optional bank-21 remaps controlled by the MMC registers.
        if address & 0x0040_0000 == 0 {
            let mut address = address;
            if address & 0x0080_0000 == 0
                && address & 0x0020_0000 != 0
                && self.banks.bank1 & 0x80 != 0
            {
                address &= !0x0020_0000;
            }
            if address & 0x0080_0000 != 0
                && address & 0x0020_0000 != 0
                && self.banks.bank3 & 0x80 != 0
            {
                address &= !0x0020_0000;
            }
            let linear = (((address >> 16) & 0x3F) << 15) | (address & 0x7FFF);
            return rom.get(linear as usize).copied().unwrap_or(0);
        }

        // $C0-$FF:0000-$FFFF: a decompressing DMA reads through here.
        if self.r4800 & self.r4801 != 0 {
            for n in 0..8usize {
                let bit = 1u8 << n;
                if self.r4800 & bit == 0 || self.r4801 & bit == 0 {
                    continue;
                }
                if address != self.dma[n].address {
                    continue;
                }
                if !self.dma_ready {
                    self.decompressor.init(address, rom, &self.banks);
                    self.dma_ready = true;
                    self.streams = self.streams.wrapping_add(1);
                }
                let data = self.decompressor.read(rom, &self.banks);
                self.bytes = self.bytes.wrapping_add(1);

                self.dma[n].size = self.dma[n].size.wrapping_sub(1);
                if self.dma[n].size == 0 {
                    self.dma_ready = false;
                    self.r4801 &= !bit;
                }
                return data;
            }
        }

        self.mmc_read(address, rom)
    }

    /// Save-RAM index for an S-DD1 board address, if it is in either SRAM
    /// window: `$00-$3F,$80-$BF:$6000-$7FFF` or `$70-$73:$0000-$FFFF`.
    ///
    /// ares maps the 8 KiB of SRAM with `mask=0xe000` (and `0x8000` for the
    /// `$70-$73` window), so the low 13 bits select the byte.
    pub const fn sram_index(address: u32) -> Option<usize> {
        let bank = (address >> 16) & 0xFF;
        let offset = (address & 0xFFFF) as usize;
        if bank >= 0x70 && bank <= 0x73 {
            return Some(offset & 0x1FFF);
        }
        if (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x6000 && offset <= 0x7FFF
        {
            return Some(offset & 0x1FFF);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_round_trip_and_mask() {
        let rom = vec![0u8; 0x10_0000];
        let mut sdd1 = Sdd1::new();
        assert_eq!((sdd1.r4800, sdd1.r4801), (0, 0));
        assert_eq!(
            (
                sdd1.banks.bank0,
                sdd1.banks.bank1,
                sdd1.banks.bank2,
                sdd1.banks.bank3
            ),
            (0, 1, 2, 3)
        );

        sdd1.write_io(0x4800, 0xA5);
        sdd1.write_io(0x4801, 0x5A);
        assert_eq!(sdd1.read_io(0x4800, &rom), 0xA5);
        assert_eq!(sdd1.read_io(0x4801, &rom), 0x5A);

        // Bank registers keep only bits 0-3 and 7.
        sdd1.write_io(0x4804, 0xFF);
        sdd1.write_io(0x4805, 0x8F);
        sdd1.write_io(0x4806, 0x00);
        sdd1.write_io(0x4807, 0x7F);
        assert_eq!(sdd1.banks.bank0, 0x8F);
        assert_eq!(sdd1.banks.bank1, 0x8F);
        assert_eq!(sdd1.banks.bank2, 0x00);
        assert_eq!(sdd1.banks.bank3, 0x0F);
        assert_eq!(sdd1.read_io(0x4807, &rom), 0x0F);

        // $4802/$4803 and $4808-$480F fall through to the raw ROM offset.
        let mut rom = vec![0u8; 0x10_0000];
        rom[0x4802] = 0x11;
        rom[0x480B] = 0x22;
        assert_eq!(sdd1.read_io(0x4802, &rom), 0x11);
        assert_eq!(sdd1.read_io(0x480B, &rom), 0x22);
    }

    #[test]
    fn classifies_the_bus_windows() {
        // Registers: $00-$3F,$80-$BF:$4800-$480F.
        assert!(Sdd1::in_io_window(0x00_4800));
        assert!(Sdd1::in_io_window(0xBF_480F));
        assert!(!Sdd1::in_io_window(0x40_4800));
        assert!(!Sdd1::in_io_window(0x00_4810));

        // MCU: $00-$3F,$80-$BF:$8000-$FFFF and $C0-$FF:$0000-$FFFF.
        assert!(Sdd1::in_mcu_window(0x00_8000));
        assert!(Sdd1::in_mcu_window(0x3F_FFFF));
        assert!(Sdd1::in_mcu_window(0x80_8000));
        assert!(Sdd1::in_mcu_window(0xC0_0000));
        assert!(Sdd1::in_mcu_window(0xFF_FFFF));
        assert!(!Sdd1::in_mcu_window(0x00_7FFF));
        assert!(!Sdd1::in_mcu_window(0x40_8000));

        // SRAM: $00-$3F,$80-$BF:$6000-$7FFF and $70-$73:$0000-$FFFF.
        assert_eq!(Sdd1::sram_index(0x00_6000), Some(0));
        assert_eq!(Sdd1::sram_index(0x00_7FFF), Some(0x1FFF));
        assert_eq!(Sdd1::sram_index(0x80_6000), Some(0));
        assert_eq!(Sdd1::sram_index(0x70_0000), Some(0));
        assert_eq!(Sdd1::sram_index(0x73_FFFF), Some(0x1FFF));
        assert_eq!(Sdd1::sram_index(0x00_8000), None);
        assert_eq!(Sdd1::sram_index(0xC0_6000), None);
    }

    #[test]
    fn dma_snoop_captures_address_and_size() {
        let mut sdd1 = Sdd1::new();
        // Channel 3: source $DA1C01, size 2048.
        sdd1.note_dma_write(0x4332, 0x01);
        sdd1.note_dma_write(0x4333, 0x1C);
        sdd1.note_dma_write(0x4334, 0xDA);
        sdd1.note_dma_write(0x4335, 0x00);
        sdd1.note_dma_write(0x4336, 0x08);
        assert_eq!(sdd1.dma[3].address, 0x00DA_1C01);
        assert_eq!(sdd1.dma[3].size, 0x0800);
        // Other channels are untouched.
        assert_eq!(sdd1.dma[0], Sdd1Dma::default());
    }

    #[test]
    fn lorom_fold_and_a21_remap() {
        let mut rom = vec![0u8; 0x20_0000];
        rom[0] = 0x11;
        rom[0x10_0000] = 0x22;
        let mut sdd1 = Sdd1::new();

        // $00-$3F,$80-$BF:$8000-$FFFF fold like LoROM.
        assert_eq!(sdd1.mcu_read(0x00_8000, &rom), 0x11);
        assert_eq!(sdd1.mcu_read(0x80_8000, &rom), 0x11);
        assert_eq!(sdd1.mcu_read(0x20_8000, &rom), 0x22);

        // r4805 bit 7 folds $20-$3F onto $00-$1F.
        sdd1.write_io(0x4805, 0x81);
        assert_eq!(sdd1.mcu_read(0x20_8000, &rom), 0x11);

        // r4807 bit 7 folds $A0-$BF onto $80-$9F.
        sdd1.write_io(0x4807, 0x83);
        assert_eq!(sdd1.mcu_read(0xA0_8000, &rom), 0x11);
    }

    #[test]
    fn mmc_banks_select_the_high_nibble() {
        let mut rom = vec![0u8; 0x80_0000];
        for bank in 0..8usize {
            rom[bank << 20] = bank as u8;
        }
        let mut sdd1 = Sdd1::new();
        sdd1.write_io(0x4804, 0x02);
        sdd1.write_io(0x4805, 0x03);
        sdd1.write_io(0x4806, 0x04);
        sdd1.write_io(0x4807, 0x05);
        assert_eq!(sdd1.mmc_read(0xC0_0000, &rom), 0x02);
        assert_eq!(sdd1.mmc_read(0xD0_0000, &rom), 0x03);
        assert_eq!(sdd1.mmc_read(0xE0_0000, &rom), 0x04);
        assert_eq!(sdd1.mmc_read(0xF0_0000, &rom), 0x05);
        // Bits 4-6 are ignored by the bank registers.
        sdd1.write_io(0x4804, 0x72);
        assert_eq!(sdd1.mmc_read(0xC0_0000, &rom), 0x02);
    }

    #[test]
    fn streaming_decompression_consumes_the_dma() {
        // An all-zero ROM decodes a blank (all-zero) image; the point of the
        // test is the host handshake around the stream.
        let rom = vec![0u8; 0x100];
        let mut sdd1 = Sdd1::new();
        sdd1.write_io(0x4800, 0x01);
        sdd1.write_io(0x4801, 0x01);
        sdd1.note_dma_write(0x4302, 0x00);
        sdd1.note_dma_write(0x4303, 0x00);
        sdd1.note_dma_write(0x4304, 0xC0);
        sdd1.note_dma_write(0x4305, 0x04);
        sdd1.note_dma_write(0x4306, 0x00);

        assert!(!sdd1.dma_ready);
        for i in 0..4 {
            let byte = sdd1.mcu_read(0xC0_0000, &rom);
            assert_eq!(byte, 0);
            assert_eq!(sdd1.dma_ready, i != 3, "stream should finish on byte 4");
        }
        assert_eq!(sdd1.r4801, 0x00);
        assert_eq!(sdd1.bytes, 4);
        assert_eq!(sdd1.streams, 1);
        // Once the channel is disabled, reads go back to the MMC bank window.
        assert_eq!(sdd1.mcu_read(0xC0_0000, &rom), 0);
    }
}
