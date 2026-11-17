//! SPC7110 coprocessor (Hudson Soft / NEC).
//!
//! Ported from the ares `SPC7110` component. The chip adds four units to a
//! HiROM board:
//!
//! - a **memory control unit** that banks the program ROM and the separate
//!   data ROM into the cartridge windows, plus an optional 8 KiB save RAM,
//! - a **decompression unit** (DCU) that decodes `q_zip`-compressed tiles into
//!   the `$4800` data port,
//! - a **data port unit** that streams bytes out of the data ROM with offset,
//!   adjust and stride registers,
//! - a **16-bit multiply / 32-bit divide ALU**.
//!
//! The decompressor is the neviksti/talarubi arithmetic coder from ares; it is
//! bit-exact or the tiles come out as noise.
//!
//! Firmware is not involved: the program lives in the cartridge's program ROM
//! (the first MiB) and the tables/compressed data in its data ROM (the rest of
//! the image), so the board works with just the game.

use crate::epsonrtc::EpsonRtc;

/// Program ROM size for a `SPC7110-` board (ares/mia `promSize`).
pub const PROM_SIZE: usize = 0x10_0000;

/// Save RAM size for `SPC7110-RAM` boards.
pub const RAM_SIZE: usize = 0x2000;

/// One probability-model entry (ares `Decompressor::ModelState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct ModelState {
    /// Probability of the more-probable symbol, scaled to 8 bits.
    probability: u8,
    /// Next model state after emitting `{MPS, LPS}`.
    next: [u8; 2],
}

const fn model(probability: u8, mps: u8, lps: u8) -> ModelState {
    ModelState {
        probability,
        next: [mps, lps],
    }
}

/// The 53-state evolution table (ares `Decompressor::evolution`).
const EVOLUTION: [ModelState; 53] = [
    model(0x5a, 1, 1),
    model(0x25, 2, 6),
    model(0x11, 3, 8),
    model(0x08, 4, 10),
    model(0x03, 5, 12),
    model(0x01, 5, 15),
    model(0x5a, 7, 7),
    model(0x3f, 8, 19),
    model(0x2c, 9, 21),
    model(0x20, 10, 22),
    model(0x17, 11, 23),
    model(0x11, 12, 25),
    model(0x0c, 13, 26),
    model(0x09, 14, 28),
    model(0x07, 15, 29),
    model(0x05, 16, 31),
    model(0x04, 17, 32),
    model(0x03, 18, 34),
    model(0x02, 5, 35),
    model(0x5a, 20, 20),
    model(0x48, 21, 39),
    model(0x3a, 22, 40),
    model(0x2e, 23, 42),
    model(0x26, 24, 44),
    model(0x1f, 25, 45),
    model(0x19, 26, 46),
    model(0x15, 27, 25),
    model(0x11, 28, 26),
    model(0x0e, 29, 26),
    model(0x0b, 30, 27),
    model(0x09, 31, 28),
    model(0x08, 32, 29),
    model(0x07, 33, 30),
    model(0x05, 34, 31),
    model(0x04, 35, 33),
    model(0x04, 36, 33),
    model(0x03, 37, 34),
    model(0x02, 38, 35),
    model(0x02, 5, 36),
    model(0x58, 40, 39),
    model(0x4d, 41, 47),
    model(0x43, 42, 48),
    model(0x3b, 43, 49),
    model(0x34, 44, 50),
    model(0x2e, 45, 51),
    model(0x29, 46, 44),
    model(0x25, 24, 45),
    model(0x56, 48, 47),
    model(0x4f, 49, 47),
    model(0x47, 50, 48),
    model(0x41, 51, 49),
    model(0x3c, 52, 50),
    model(0x37, 43, 51),
];

/// One adaptive context: its model state and MPS/LPS role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct Context {
    /// Index into [`EVOLUTION`].
    prediction: u8,
    /// Non-zero swaps the meaning of MPS and LPS.
    swap: u8,
}

/// ares `Bus::mirror`: fold `address` into `size` bytes.
fn mirror(mut address: u32, mut size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    address &= 0x00FF_FFFF;
    let mut base = 0u32;
    let mut mask = 1u32 << 23;
    while address >= size {
        while address & mask == 0 {
            mask >>= 1;
        }
        address -= mask;
        if size > mask {
            size -= mask;
            base += mask;
        }
        mask >>= 1;
    }
    base + address
}

/// The `q_zip` arithmetic decompressor (ares `SPC7110::Decompressor`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Spc7110Decompressor {
    context: [[Context; 15]; 5],
    /// Bits per pixel: 1, 2 or 4.
    pub bpp: u32,
    /// Data-ROM read offset.
    offset: u32,
    /// Bits remaining in the input word.
    bits: u32,
    /// Arithmetic range (8-bit, but `Max + 1` fits 16 bits).
    range: u16,
    /// Input word from the data ROM.
    input: u16,
    /// Bits decoded for the current byte.
    output: u8,
    /// Last 64 decoded pixels.
    pixels: u64,
    /// Most-recently-used colour list.
    colormap: u64,
    /// Decompressed word after [`Self::decode`].
    pub result: u32,
}

impl Spc7110Decompressor {
    const MPS: u32 = 0;
    const LPS: u32 = 1;
    const HALF: u16 = 0x55;
    const MAX: u16 = 0xff;

    /// Inverse Morton transform: unpack big-endian packed pixels.
    ///
    /// Odd bits land in the lower half, even bits in the upper half.
    fn deinterleave(data: u64, bits: u32) -> u32 {
        let mask = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let mut data = data & mask;
        data = 0x5555_5555_5555_5555 & ((data << bits) | (data >> 1));
        data = 0x3333_3333_3333_3333 & (data | (data >> 1));
        data = 0x0f0f_0f0f_0f0f_0f0f & (data | (data >> 2));
        data = 0x00ff_00ff_00ff_00ff & (data | (data >> 4));
        data = 0x0000_ffff_0000_ffff & (data | (data >> 8));
        (data | (data >> 16)) as u32
    }

    /// Move `nibble` to the front of the most-recently-used list.
    fn move_to_front(mut list: u64, nibble: u32) -> u64 {
        let mut n = 0u32;
        let mut mask = !15u64;
        while n < 64 {
            if (list >> n) & 15 != u64::from(nibble) {
                n += 4;
                mask <<= 4;
                continue;
            }
            list = (list & mask)
                .wrapping_add((list << 4) & !mask)
                .wrapping_add(u64::from(nibble));
            return list;
        }
        list
    }

    /// Reset the coder at `origin` in the data ROM (ares `initialize`).
    fn initialize(&mut self, mode: u32, origin: u32, drom: &[u8], size_mode: u8) {
        self.context = [[Context::default(); 15]; 5];
        self.bpp = 1 << mode;
        self.offset = origin;
        self.bits = 8;
        self.range = Self::MAX + 1;
        let high = u16::from(datarom_read(drom, size_mode, self.offset));
        self.offset = self.offset.wrapping_add(1);
        let low = u16::from(datarom_read(drom, size_mode, self.offset));
        self.offset = self.offset.wrapping_add(1);
        self.input = (high << 8) | low;
        self.output = 0;
        self.pixels = 0;
        self.colormap = 0xfedc_ba98_7654_3210;
    }

    /// Decode one 8-pixel run; `result` holds the packed word afterwards.
    fn decode(&mut self, drom: &[u8], size_mode: u8) {
        for pixel in 0..8u32 {
            let mut map = self.colormap;
            let mut diff = 0u32;

            if self.bpp > 1 {
                let pa = if self.bpp == 2 {
                    ((self.pixels >> 2) & 3) as u32
                } else {
                    (self.pixels & 15) as u32
                };
                let pb = if self.bpp == 2 {
                    ((self.pixels >> 14) & 3) as u32
                } else {
                    ((self.pixels >> 28) & 15) as u32
                };
                let pc = if self.bpp == 2 {
                    ((self.pixels >> 16) & 3) as u32
                } else {
                    ((self.pixels >> 32) & 15) as u32
                };

                if pa != pb || pb != pc {
                    let matching = pa ^ pb ^ pc;
                    diff = 4; // no match; all pixels differ
                    if matching ^ pc == 0 {
                        diff = 3; // a == b; pixel c differs
                    }
                    if matching ^ pb == 0 {
                        diff = 2; // c == a; pixel b differs
                    }
                    if matching ^ pa == 0 {
                        diff = 1; // b == c; pixel a differs
                    }
                }

                self.colormap = Self::move_to_front(self.colormap, pa);

                map = Self::move_to_front(map, pc);
                map = Self::move_to_front(map, pb);
                map = Self::move_to_front(map, pa);
            }

            for plane in 0..self.bpp {
                let bit = if self.bpp > 1 {
                    1u32 << plane
                } else {
                    1u32 << (pixel & 3)
                };
                let history = (bit - 1) & u32::from(self.output);
                let mut set = 0usize;
                if self.bpp == 1 {
                    set = usize::from(pixel >= 4);
                }
                if self.bpp == 2 {
                    set = diff as usize;
                }
                if plane >= 2 && history <= 1 {
                    set = diff as usize;
                }

                let slot = (bit + history - 1) as usize;
                let ctx = &mut self.context[set.min(4)][slot.min(14)];
                let state = EVOLUTION[usize::from(ctx.prediction) & 63];
                let lps = self.range.wrapping_sub(u16::from(state.probability));
                // Only the MSB of the shifted range is compared.
                let symbol = u32::from(self.input) >= (u32::from(lps) << 8);
                let symbol = u32::from(symbol);

                self.output = (self.output << 1) | ((symbol as u8) ^ ctx.swap);

                if symbol == Self::MPS {
                    // [0 ... range-p]
                    self.range = lps;
                } else {
                    // [range-p+1 ... range]
                    self.range = self.range.wrapping_sub(lps);
                    self.input = self.input.wrapping_sub(lps << 8);
                }

                while self.range <= Self::MAX / 2 {
                    ctx.prediction = state.next[symbol as usize];
                    self.range <<= 1;
                    self.input <<= 1;
                    self.bits -= 1;
                    if self.bits == 0 {
                        self.bits = 8;
                        let byte = datarom_read(drom, size_mode, self.offset);
                        self.offset = self.offset.wrapping_add(1);
                        self.input = self.input.wrapping_add(u16::from(byte));
                    }
                }

                if symbol == Self::LPS && u16::from(state.probability) > Self::HALF {
                    ctx.swap ^= 1;
                }
            }

            let index = u32::from(self.output) & ((1 << self.bpp) - 1);
            let index = if self.bpp == 1 {
                index ^ ((self.pixels >> 15) as u32 & 1)
            } else {
                index
            };

            let nibble = (map >> (4 * index)) & 15;
            self.pixels = (self.pixels << self.bpp) | nibble;
        }

        self.result = match self.bpp {
            1 => self.pixels as u32,
            2 => Self::deinterleave(self.pixels, 16),
            _ => Self::deinterleave(u64::from(Self::deinterleave(self.pixels, 32)), 32),
        };
    }
}

/// Read the data ROM with the MCU's size mapping (ares `dataromRead`).
fn datarom_read(drom: &[u8], size_mode: u8, address: u32) -> u8 {
    let size = 1u32 << (size_mode & 3); // size in MiB
    let mask = 0x10_0000 * size - 1;
    let offset = address & mask;
    if size_mode & 3 != 3 && address & 0x40_0000 != 0 {
        return 0;
    }
    if drom.is_empty() {
        return 0;
    }
    drom[mirror(offset, drom.len() as u32) as usize]
}

/// The SPC7110 coprocessor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Spc7110 {
    /// Program ROM (the first MiB of the image). Not serialized; it is
    /// re-attached from the cartridge on load.
    #[serde(skip)]
    pub prom: Vec<u8>,
    /// Data ROM (the rest of the image). Not serialized for the same reason.
    #[serde(skip)]
    pub drom: Vec<u8>,
    /// Save RAM.
    pub ram: Vec<u8>,

    // Decompression unit.
    r4801: u8,
    r4802: u8,
    r4803: u8,
    r4804: u8,
    r4805: u8,
    r4806: u8,
    r4807: u8,
    r4809: u8,
    r480a: u8,
    r480b: u8,
    /// `$480c` decompression status (debug).
    pub r480c: u8,
    dcu_pending: bool,
    /// Last DCU mode selected by `$4804` (debug).
    pub dcu_mode: u32,
    dcu_address: u32,
    dcu_offset: u32,
    dcu_tile: [u8; 32],
    /// The `q_zip` decoder.
    pub decompressor: Spc7110Decompressor,

    // Data port unit.
    r4810: u8,
    r4811: u8,
    r4812: u8,
    r4813: u8,
    r4814: u8,
    r4815: u8,
    r4816: u8,
    r4817: u8,
    r4818: u8,
    r481a: u8,

    // Arithmetic logic unit.
    r4820: u8,
    r4821: u8,
    r4822: u8,
    r4823: u8,
    r4824: u8,
    r4825: u8,
    r4826: u8,
    r4827: u8,
    r4828: u8,
    r4829: u8,
    r482a: u8,
    r482b: u8,
    r482c: u8,
    r482d: u8,
    r482e: u8,
    r482f: u8,
    mul_pending: bool,
    div_pending: bool,

    // Memory control unit.
    r4830: u8,
    r4831: u8,
    r4832: u8,
    r4833: u8,
    /// `$4834` MCU mapping/size (debug).
    pub r4834: u8,

    /// The Epson RTC-4513 on `$F9` boards (`$4840-$4842`).
    pub rtc: Option<EpsonRtc>,
    /// RTC oscillator credit, in 1/2^21 s units.
    rtc_credit: i64,

    /// Master clocks consumed by pending-unit work (debug/timing).
    pub busy_clocks: u64,
}

impl Spc7110 {
    /// Build a powered-on chip from the cartridge image.
    ///
    /// ares/mia split a `SPC7110-` image into `program.rom` (the first MiB)
    /// and `data.rom` (everything after it).
    pub fn new(rom: &[u8], sram_size: usize, has_rtc: bool) -> Self {
        let prom_len = PROM_SIZE.min(rom.len());
        let mut chip = Self {
            prom: rom[..prom_len].to_vec(),
            drom: rom[prom_len..].to_vec(),
            ram: vec![0; sram_size.max(RAM_SIZE)],
            r4801: 0,
            r4802: 0,
            r4803: 0,
            r4804: 0,
            r4805: 0,
            r4806: 0,
            r4807: 0,
            r4809: 0,
            r480a: 0,
            r480b: 0,
            r480c: 0,
            dcu_pending: false,
            dcu_mode: 0,
            dcu_address: 0,
            dcu_offset: 0,
            dcu_tile: [0; 32],
            decompressor: Spc7110Decompressor::default(),
            r4810: 0,
            r4811: 0,
            r4812: 0,
            r4813: 0,
            r4814: 0,
            r4815: 0,
            r4816: 0,
            r4817: 0,
            r4818: 0,
            r481a: 0,
            r4820: 0,
            r4821: 0,
            r4822: 0,
            r4823: 0,
            r4824: 0,
            r4825: 0,
            r4826: 0,
            r4827: 0,
            r4828: 0,
            r4829: 0,
            r482a: 0,
            r482b: 0,
            r482c: 0,
            r482d: 0,
            r482e: 0,
            r482f: 0,
            mul_pending: false,
            div_pending: false,
            r4830: 0,
            r4831: 0,
            r4832: 1,
            r4833: 2,
            r4834: 0,
            rtc: if has_rtc { Some(EpsonRtc::new()) } else { None },
            rtc_credit: 0,
            busy_clocks: 0,
        };
        chip.power();
        chip
    }

    /// Re-attach the cartridge image after a save-state load.
    pub fn attach_rom(&mut self, rom: &[u8]) {
        let prom_len = PROM_SIZE.min(rom.len());
        self.prom.clear();
        self.prom.extend_from_slice(&rom[..prom_len]);
        self.drom.clear();
        self.drom.extend_from_slice(&rom[prom_len..]);
    }

    /// Reset the register file (ares `SPC7110::power`). ROM and RAM persist.
    pub fn power(&mut self) {
        self.r4801 = 0;
        self.r4802 = 0;
        self.r4803 = 0;
        self.r4804 = 0;
        self.r4805 = 0;
        self.r4806 = 0;
        self.r4807 = 0;
        self.r4809 = 0;
        self.r480a = 0;
        self.r480b = 0;
        self.r480c = 0;
        self.dcu_pending = false;
        self.dcu_mode = 0;
        self.dcu_address = 0;
        self.dcu_offset = 0;
        self.r4810 = 0;
        self.r4811 = 0;
        self.r4812 = 0;
        self.r4813 = 0;
        self.r4814 = 0;
        self.r4815 = 0;
        self.r4816 = 0;
        self.r4817 = 0;
        self.r4818 = 0;
        self.r481a = 0;
        self.r4820 = 0;
        self.r4821 = 0;
        self.r4822 = 0;
        self.r4823 = 0;
        self.r4824 = 0;
        self.r4825 = 0;
        self.r4826 = 0;
        self.r4827 = 0;
        self.r4828 = 0;
        self.r4829 = 0;
        self.r482a = 0;
        self.r482b = 0;
        self.r482c = 0;
        self.r482d = 0;
        self.r482e = 0;
        self.r482f = 0;
        self.mul_pending = false;
        self.div_pending = false;
        self.r4830 = 0;
        self.r4831 = 0;
        self.r4832 = 1;
        self.r4833 = 2;
        self.r4834 = 0;
        if let Some(rtc) = self.rtc.as_mut() {
            rtc.power();
        }
        self.rtc_credit = 0;
    }

    // --- Bus windows ------------------------------------------------------

    /// The register window `$00-$3F,$80-$BF:$4800-$483F`, the `$50`/`$58`
    /// full-bank aliases ares maps, and the Epson RTC ports at `$4840-$4842`
    /// (encoded as `$40-$42`).
    pub const fn register_offset(address: u32) -> Option<usize> {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        if bank == 0x50 {
            return Some(0x00);
        }
        if bank == 0x58 {
            return Some(0x08);
        }
        if bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF) {
            if offset >= 0x4800 && offset <= 0x483F {
                return Some(offset & 0x3F);
            }
            if offset >= 0x4840 && offset <= 0x4842 {
                return Some(0x40 + (offset - 0x4840));
            }
        }
        None
    }

    /// Whether `address` hits the MCU ROM windows
    /// (`$00-$3F,$80-$BF:$8000-$FFFF` or `$C0-$FF:$0000-$FFFF`).
    pub const fn in_rom_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        if bank >= 0xC0 {
            return true;
        }
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && (address & 0xFFFF) >= 0x8000
    }

    /// ares' `mask=0x800000` / `mask=0xc00000` reduction: drop bits 22-23.
    pub const fn reduce_rom(address: u32) -> u32 {
        let bank = (address >> 16) & 0x3F;
        (bank << 16) | (address & 0xFFFF)
    }

    /// The save-RAM window `$00-$3F,$80-$BF:$6000-$7FFF` folded to 8 KiB.
    pub const fn ram_index(address: u32) -> Option<usize> {
        let bank = (address >> 16) & 0xFF;
        let offset = (address & 0xFFFF) as usize;
        if (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x6000 && offset <= 0x7FFF
        {
            return Some(offset & 0x1FFF);
        }
        None
    }

    // --- Registers --------------------------------------------------------

    /// Read the `$4800-$483F` register window (ares `SPC7110::read`).
    pub fn read(&mut self, offset: usize, data: u8) -> u8 {
        if (0x40..=0x42).contains(&offset) {
            return match self.rtc.as_mut() {
                Some(rtc) => rtc.read(offset - 0x40, data),
                None => data,
            };
        }
        match 0x4800 | (offset & 0x3F) {
            // Decompression unit.
            0x4800 => {
                let counter =
                    (u16::from(self.r4809) | (u16::from(self.r480a) << 8)).wrapping_sub(1);
                self.r4809 = counter as u8;
                self.r480a = (counter >> 8) as u8;
                self.dcu_read()
            }
            0x4801 => self.r4801,
            0x4802 => self.r4802,
            0x4803 => self.r4803,
            0x4804 => self.r4804,
            0x4805 => self.r4805,
            0x4806 => self.r4806,
            0x4807 => self.r4807,
            0x4808 => 0x00,
            0x4809 => self.r4809,
            0x480A => self.r480a,
            0x480B => self.r480b,
            0x480C => self.r480c,

            // Data port unit.
            0x4810 => {
                let value = self.r4810;
                self.data_port_increment_4810();
                value
            }
            0x4811 => self.r4811,
            0x4812 => self.r4812,
            0x4813 => self.r4813,
            0x4814 => self.r4814,
            0x4815 => self.r4815,
            0x4816 => self.r4816,
            0x4817 => self.r4817,
            0x4818 => self.r4818,
            0x481A => {
                self.data_port_increment_481a();
                0x00
            }

            // Arithmetic logic unit.
            0x4820 => self.r4820,
            0x4821 => self.r4821,
            0x4822 => self.r4822,
            0x4823 => self.r4823,
            0x4824 => self.r4824,
            0x4825 => self.r4825,
            0x4826 => self.r4826,
            0x4827 => self.r4827,
            0x4828 => self.r4828,
            0x4829 => self.r4829,
            0x482A => self.r482a,
            0x482B => self.r482b,
            0x482C => self.r482c,
            0x482D => self.r482d,
            0x482E => self.r482e,
            0x482F => self.r482f,

            // Memory control unit.
            0x4830 => self.r4830,
            0x4831 => self.r4831,
            0x4832 => self.r4832,
            0x4833 => self.r4833,
            0x4834 => self.r4834,

            _ => data,
        }
    }

    /// Write the `$4800-$483F` register window (ares `SPC7110::write`).
    pub fn write(&mut self, offset: usize, data: u8) {
        if (0x40..=0x42).contains(&offset) {
            if let Some(rtc) = self.rtc.as_mut() {
                rtc.write(offset - 0x40, data);
            }
            return;
        }
        match 0x4800 | (offset & 0x3F) {
            // Decompression unit.
            0x4801 => self.r4801 = data,
            0x4802 => self.r4802 = data,
            0x4803 => self.r4803 = data,
            0x4804 => {
                self.r4804 = data;
                self.dcu_load_address();
            }
            0x4805 => self.r4805 = data,
            0x4806 => {
                self.r4806 = data;
                self.r480c &= 0x7F;
                self.dcu_pending = true;
            }
            0x4807 => self.r4807 = data,
            0x4808 => {}
            0x4809 => self.r4809 = data,
            0x480A => self.r480a = data,
            0x480B => self.r480b = data & 0x03,

            // Data port unit.
            0x4811 => self.r4811 = data,
            0x4812 => self.r4812 = data,
            0x4813 => {
                self.r4813 = data;
                self.data_port_read();
            }
            0x4814 => {
                self.r4814 = data;
                self.data_port_increment_4814();
            }
            0x4815 => {
                self.r4815 = data;
                if self.r4818 & 2 != 0 {
                    self.data_port_read();
                }
                self.data_port_increment_4815();
            }
            0x4816 => self.r4816 = data,
            0x4817 => self.r4817 = data,
            0x4818 => {
                self.r4818 = data & 0x7F;
                self.data_port_read();
            }

            // Arithmetic logic unit.
            0x4820 => self.r4820 = data,
            0x4821 => self.r4821 = data,
            0x4822 => self.r4822 = data,
            0x4823 => self.r4823 = data,
            0x4824 => self.r4824 = data,
            0x4825 => {
                self.r4825 = data;
                self.r482f |= 0x81;
                self.mul_pending = true;
            }
            0x4826 => self.r4826 = data,
            0x4827 => {
                self.r4827 = data;
                self.r482f |= 0x80;
                self.div_pending = true;
            }
            0x482E => self.r482e = data & 0x01,

            // Memory control unit.
            0x4830 => self.r4830 = data & 0x87,
            0x4831 => self.r4831 = data & 0x07,
            0x4832 => self.r4832 = data & 0x07,
            0x4833 => self.r4833 = data & 0x07,
            0x4834 => self.r4834 = data & 0x07,

            _ => {}
        }
    }

    // --- MCU --------------------------------------------------------------

    /// Read through the MCU ROM window (ares `mcuromRead`), `address` already
    /// reduced with [`Self::reduce_rom`].
    pub fn mcurom_read(&self, address: u32, data: u8) -> u8 {
        if address < 0x10_0000 {
            let address = address & 0x0F_FFFF;
            if !self.prom.is_empty() {
                return self.prom[mirror(address, self.prom.len() as u32) as usize];
            }
            let address = address | (0x10_0000 * u32::from(self.r4830 & 7));
            return datarom_read(&self.drom, self.r4834, address);
        }
        if address < 0x20_0000 {
            let address = address & 0x0F_FFFF;
            if self.r4834 & 4 != 0 {
                return self.prom[mirror(0x10_0000 + address, self.prom.len() as u32) as usize];
            }
            let address = address | (0x10_0000 * u32::from(self.r4831 & 7));
            return datarom_read(&self.drom, self.r4834, address);
        }
        if address < 0x30_0000 {
            let address = (address & 0x0F_FFFF) | (0x10_0000 * u32::from(self.r4832 & 7));
            return datarom_read(&self.drom, self.r4834, address);
        }
        if address < 0x40_0000 {
            let address = (address & 0x0F_FFFF) | (0x10_0000 * u32::from(self.r4833 & 7));
            return datarom_read(&self.drom, self.r4834, address);
        }
        data
    }

    /// Read the save RAM (ares `mcuramRead`); `index` is the folded 8 KiB
    /// offset.
    pub fn ram_read(&self, index: usize, data: u8) -> u8 {
        if self.r4830 & 0x80 != 0 {
            return self.ram[mirror(index as u32, self.ram.len() as u32) as usize];
        }
        let _ = data;
        0x00
    }

    /// Write the save RAM (ares `mcuramWrite`).
    pub fn ram_write(&mut self, index: usize, data: u8) {
        if self.r4830 & 0x80 != 0 {
            let index = mirror(index as u32, self.ram.len() as u32) as usize;
            self.ram[index] = data;
        }
    }

    // --- Decompression unit ----------------------------------------------

    /// ares `dcuLoadAddress`.
    fn dcu_load_address(&mut self) {
        let table =
            u32::from(self.r4801) | (u32::from(self.r4802) << 8) | (u32::from(self.r4803) << 16);
        let index = u32::from(self.r4804) << 2;

        let address = table.wrapping_add(index);
        self.dcu_mode = u32::from(datarom_read(&self.drom, self.r4834, address));
        let mut dcu_address = u32::from(datarom_read(&self.drom, self.r4834, address + 1)) << 16;
        dcu_address |= u32::from(datarom_read(&self.drom, self.r4834, address + 2)) << 8;
        dcu_address |= u32::from(datarom_read(&self.drom, self.r4834, address + 3));
        self.dcu_address = dcu_address;
    }

    /// ares `dcuBeginTransfer`. Returns the master clocks the S-CPU stalls.
    fn dcu_begin_transfer(&mut self) -> u32 {
        if self.dcu_mode == 3 {
            return 0; // invalid mode
        }
        self.decompressor
            .initialize(self.dcu_mode, self.dcu_address, &self.drom, self.r4834);
        self.decompressor.decode(&self.drom, self.r4834);

        let mut seek = if self.r480b & 2 != 0 {
            u16::from(self.r4805) | (u16::from(self.r4806) << 8)
        } else {
            0
        };
        while seek != 0 {
            seek -= 1;
            self.decompressor.decode(&self.drom, self.r4834);
        }

        self.r480c |= 0x80;
        self.dcu_offset = 0;
        20
    }

    /// ares `dcuRead`.
    fn dcu_read(&mut self) -> u8 {
        if self.r480c & 0x80 == 0 {
            return 0x00;
        }

        if self.dcu_offset == 0 {
            for row in 0..8u32 {
                let result = self.decompressor.result;
                match self.decompressor.bpp {
                    1 => self.dcu_tile[row as usize] = result as u8,
                    2 => {
                        self.dcu_tile[row as usize * 2] = result as u8;
                        self.dcu_tile[row as usize * 2 + 1] = (result >> 8) as u8;
                    }
                    _ => {
                        self.dcu_tile[row as usize * 2] = result as u8;
                        self.dcu_tile[row as usize * 2 + 1] = (result >> 8) as u8;
                        self.dcu_tile[row as usize * 2 + 16] = (result >> 16) as u8;
                        self.dcu_tile[row as usize * 2 + 17] = (result >> 24) as u8;
                    }
                }

                let mut seek = if self.r480b & 1 != 0 { self.r4807 } else { 1 };
                while seek != 0 {
                    seek -= 1;
                    self.decompressor.decode(&self.drom, self.r4834);
                }
            }
        }

        let value = self.dcu_tile[(self.dcu_offset & 31) as usize];
        self.dcu_offset += 1;
        let bpp = self.decompressor.bpp;
        if bpp != 0 {
            self.dcu_offset &= 8 * bpp - 1;
        }
        value
    }

    // --- Data port unit ---------------------------------------------------

    fn data_offset(&self) -> u32 {
        u32::from(self.r4811) | (u32::from(self.r4812) << 8) | (u32::from(self.r4813) << 16)
    }

    fn data_adjust(&self) -> u16 {
        u16::from(self.r4814) | (u16::from(self.r4815) << 8)
    }

    fn data_stride(&self) -> u16 {
        u16::from(self.r4816) | (u16::from(self.r4817) << 8)
    }

    fn set_data_offset(&mut self, address: u32) {
        self.r4811 = address as u8;
        self.r4812 = (address >> 8) as u8;
        self.r4813 = (address >> 16) as u8;
    }

    fn set_data_adjust(&mut self, address: u16) {
        self.r4814 = address as u8;
        self.r4815 = (address >> 8) as u8;
    }

    fn data_port_read(&mut self) {
        let offset = self.data_offset();
        let mut adjust: u32 = if self.r4818 & 2 != 0 {
            u32::from(self.data_adjust())
        } else {
            0
        };
        if self.r4818 & 8 != 0 {
            adjust = adjust as u16 as i16 as u32;
        }
        self.r4810 = datarom_read(&self.drom, self.r4834, offset.wrapping_add(adjust));
    }

    fn data_port_increment_4810(&mut self) {
        let offset = self.data_offset();
        let mut stride: u32 = if self.r4818 & 1 != 0 {
            u32::from(self.data_stride())
        } else {
            1
        };
        let mut adjust: u32 = u32::from(self.data_adjust());
        if self.r4818 & 4 != 0 {
            stride = (stride as u16 as i16 as i32) as u32;
        }
        if self.r4818 & 8 != 0 {
            adjust = (adjust as u16 as i16 as i32) as u32;
        }
        if self.r4818 & 16 == 0 {
            self.set_data_offset(offset.wrapping_add(stride));
        } else {
            self.set_data_adjust(adjust.wrapping_add(stride) as u16);
        }
        self.data_port_read();
    }

    fn data_port_increment_4814(&mut self) {
        if self.r4818 >> 5 != 1 {
            return;
        }
        let offset = self.data_offset();
        let mut adjust = u32::from(self.data_adjust());
        if self.r4818 & 8 != 0 {
            adjust = (adjust as u16 as i16 as i32) as u32;
        }
        self.set_data_offset(offset.wrapping_add(adjust));
        self.data_port_read();
    }

    fn data_port_increment_4815(&mut self) {
        if self.r4818 >> 5 != 2 {
            return;
        }
        let offset = self.data_offset();
        let mut adjust = u32::from(self.data_adjust());
        if self.r4818 & 8 != 0 {
            adjust = (adjust as u16 as i16 as i32) as u32;
        }
        self.set_data_offset(offset.wrapping_add(adjust));
        self.data_port_read();
    }

    fn data_port_increment_481a(&mut self) {
        if self.r4818 >> 5 != 3 {
            return;
        }
        let offset = self.data_offset();
        let mut adjust = u32::from(self.data_adjust());
        if self.r4818 & 8 != 0 {
            adjust = (adjust as u16 as i16 as i32) as u32;
        }
        self.set_data_offset(offset.wrapping_add(adjust));
        self.data_port_read();
    }

    // --- Arithmetic logic unit -------------------------------------------

    /// ares `aluMultiply`. Returns the master clocks the S-CPU stalls.
    fn alu_multiply(&mut self) -> u32 {
        let r0 = u16::from(self.r4824) | (u16::from(self.r4825) << 8);
        let r1 = u16::from(self.r4820) | (u16::from(self.r4821) << 8);
        let result = if self.r482e & 1 != 0 {
            (i32::from(r0 as i16).wrapping_mul(i32::from(r1 as i16))) as u32
        } else {
            u32::from(r0) * u32::from(r1)
        };
        self.r4828 = result as u8;
        self.r4829 = (result >> 8) as u8;
        self.r482a = (result >> 16) as u8;
        self.r482b = (result >> 24) as u8;
        self.r482f &= 0x7F;
        30
    }

    /// ares `aluDivide`. Returns the master clocks the S-CPU stalls.
    fn alu_divide(&mut self) -> u32 {
        let dividend = u32::from(self.r4820)
            | (u32::from(self.r4821) << 8)
            | (u32::from(self.r4822) << 16)
            | (u32::from(self.r4823) << 24);
        let divisor = u16::from(self.r4826) | (u16::from(self.r4827) << 8);

        let (quotient, remainder) = if self.r482e & 1 != 0 {
            let dividend = dividend as i32;
            let divisor = divisor as i16;
            if divisor != 0 {
                (
                    dividend.wrapping_div(i32::from(divisor)) as u32,
                    dividend.wrapping_rem(i32::from(divisor)) as u16,
                )
            } else {
                (0, dividend as u16)
            }
        } else if divisor != 0 {
            (
                dividend / u32::from(divisor),
                (dividend % u32::from(divisor)) as u16,
            )
        } else {
            (0, dividend as u16)
        };

        self.r4828 = quotient as u8;
        self.r4829 = (quotient >> 8) as u8;
        self.r482a = (quotient >> 16) as u8;
        self.r482b = (quotient >> 24) as u8;
        self.r482c = remainder as u8;
        self.r482d = (remainder >> 8) as u8;
        self.r482f &= 0x7F;
        40
    }

    /// Advance the chip by `clocks` master clocks (ares `SPC7110::main`).
    ///
    /// Each pending unit runs on the next tick and then stalls the S-CPU for
    /// its latency (20 clocks for a DCU transfer, 30/40 for multiply/divide).
    /// Returns the stall clocks so the system can charge them to the CPU.
    pub fn run(&mut self, clocks: u32) -> u32 {
        if let Some(rtc) = self.rtc.as_mut() {
            // ares runs the RTC at 32,768 * 64 Hz; tick it with a credit.
            self.rtc_credit += i64::from(clocks) * i64::from(crate::epsonrtc::RTC_CLOCK);
            let den = crate::rom::Region::Ntsc.master_clock() as i64;
            while self.rtc_credit >= den {
                rtc.tick();
                self.rtc_credit -= den;
            }
        }
        let mut spent = 0u32;
        let mut extra = 0u32;
        while spent < clocks {
            spent += 1;
            let cost = if self.dcu_pending {
                self.dcu_pending = false;
                self.dcu_begin_transfer()
            } else if self.mul_pending {
                self.mul_pending = false;
                self.alu_multiply()
            } else if self.div_pending {
                self.div_pending = false;
                self.alu_divide()
            } else {
                0
            };
            if cost != 0 {
                spent += cost;
                extra += cost;
                self.busy_clocks += u64::from(cost);
            }
        }
        extra
    }

    /// Whether the cartridge header declares an SPC7110.
    ///
    /// mia selects an `SPC7110-` board from cartridge type `$F5` (and the
    /// `SPC7110-` HiROM map modes `$2A`/`$3A`).
    pub const fn header_has_spc7110(cartridge_type: u8, map_mode: u8) -> bool {
        matches!(cartridge_type, 0xf5 | 0xf9) || matches!(map_mode, 0x2a | 0x3a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chip() -> Spc7110 {
        // 1 MiB program ROM + 2 MiB data ROM, like a real board.
        let mut rom = vec![0u8; PROM_SIZE + 0x20_0000];
        for (i, byte) in rom.iter_mut().enumerate() {
            *byte = (i / 0x10_0000) as u8;
        }
        Spc7110::new(&rom, RAM_SIZE, false)
    }

    #[test]
    fn splits_the_image_into_program_and_data_rom() {
        let chip = make_chip();
        assert_eq!(chip.prom.len(), PROM_SIZE);
        assert_eq!(chip.drom.len(), 0x20_0000);
        assert_eq!(chip.prom[0], 0);
        assert_eq!(chip.drom[0], 1);
    }

    #[test]
    fn register_round_trip_and_masking() {
        let mut chip = make_chip();
        chip.write(0x01, 0xA5);
        assert_eq!(chip.read(0x01, 0), 0xA5);
        // $4830 keeps bits 0-2 and 7.
        chip.write(0x30, 0xFF);
        assert_eq!(chip.read(0x30, 0), 0x87);
        // $4834 keeps bits 0-2.
        chip.write(0x34, 0xF9);
        assert_eq!(chip.read(0x34, 0), 0x01);
        // $4808 always reads zero and $480B masks to two bits.
        chip.write(0x08, 0xFF);
        assert_eq!(chip.read(0x08, 0), 0x00);
        chip.write(0x0B, 0xFF);
        assert_eq!(chip.read(0x0B, 0), 0x03);
        // Power-on bank defaults.
        let fresh = make_chip();
        assert_eq!((fresh.r4831, fresh.r4832, fresh.r4833), (0, 1, 2));
    }

    #[test]
    fn classifies_the_bus_windows() {
        assert_eq!(Spc7110::register_offset(0x00_4800), Some(0x00));
        assert_eq!(Spc7110::register_offset(0xBF_483F), Some(0x3F));
        // $50 and $58 alias the whole bank onto $4800 / $4808.
        assert_eq!(Spc7110::register_offset(0x50_1234), Some(0x00));
        assert_eq!(Spc7110::register_offset(0x58_ABCD), Some(0x08));
        // $4840-$4842 are the Epson RTC ports, encoded as $40-$42.
        assert_eq!(Spc7110::register_offset(0x00_4840), Some(0x40));
        assert_eq!(Spc7110::register_offset(0x40_4800), None);

        assert!(Spc7110::in_rom_window(0x00_8000));
        assert!(Spc7110::in_rom_window(0x3F_FFFF));
        assert!(Spc7110::in_rom_window(0xC0_0000));
        assert!(Spc7110::in_rom_window(0xFF_FFFF));
        assert!(!Spc7110::in_rom_window(0x00_7FFF));
        // The MCU drops address bits 22-23, so $80-$BF folds onto $00-$3F.
        assert_eq!(Spc7110::reduce_rom(0x00_8000), 0x00_8000);
        assert_eq!(Spc7110::reduce_rom(0x80_8000), 0x00_8000);
        assert_eq!(Spc7110::reduce_rom(0xC0_0000), 0x00_0000);
        assert_eq!(Spc7110::reduce_rom(0xC1_0000), 0x01_0000);
        assert_eq!(Spc7110::reduce_rom(0xFF_FFFF), 0x3F_FFFF);

        // Save RAM folds $6000-$7FFF onto 8 KiB.
        assert_eq!(Spc7110::ram_index(0x00_6000), Some(0x0000));
        assert_eq!(Spc7110::ram_index(0x00_7FFF), Some(0x1FFF));
        assert_eq!(Spc7110::ram_index(0x80_6000), Some(0x0000));
        assert_eq!(Spc7110::ram_index(0x00_8000), None);
    }

    #[test]
    fn sram_is_gated_by_r4830_bit7() {
        let mut chip = make_chip();
        chip.ram_write(0x10, 0x5A);
        assert_eq!(
            chip.ram_read(0x10, 0),
            0x00,
            "writes are ignored while disabled"
        );
        chip.write(0x30, 0x80);
        chip.ram_write(0x10, 0x5A);
        assert_eq!(chip.ram_read(0x10, 0), 0x5A);
    }

    #[test]
    fn program_rom_reads_through_the_mcu() {
        let chip = make_chip();
        // Bank 0's $8000 window is program ROM at offset $8000.
        assert_eq!(chip.mcurom_read(0x00_8000, 0), 0);
        // Bank 1's window is program ROM at $18xxx.
        assert_eq!(chip.mcurom_read(0x01_8000, 0), 0);
        // Banks $10-$1F fall through to the data ROM when no 16 Mbit PROM.
        assert_eq!(chip.mcurom_read(0x10_8000, 0), 1);
        // Banks $20-$2F use the $4832 bank register (default 1).
        assert_eq!(chip.mcurom_read(0x20_8000, 0), 1);
    }

    #[test]
    fn alu_multiply_is_signed_and_unsigned() {
        let mut chip = make_chip();
        // Unsigned 3 * 5 = 15.
        chip.write(0x20, 3);
        chip.write(0x21, 0);
        chip.write(0x24, 5);
        chip.write(0x25, 0);
        chip.mul_pending = false;
        chip.alu_multiply();
        assert_eq!(
            (chip.r4828, chip.r4829, chip.r482a, chip.r482b),
            (15, 0, 0, 0)
        );
        assert_eq!(chip.read(0x2F, 0) & 0x80, 0);

        // Signed -2 * 3 = -6.
        chip.write(0x2E, 1);
        chip.write(0x20, 0xFE);
        chip.write(0x21, 0xFF);
        chip.write(0x24, 3);
        chip.write(0x25, 0);
        chip.alu_multiply();
        assert_eq!(
            (chip.r4828, chip.r4829, chip.r482a, chip.r482b),
            (0xFA, 0xFF, 0xFF, 0xFF)
        );
    }

    #[test]
    fn alu_divide_reports_quotient_and_remainder() {
        let mut chip = make_chip();
        // 100 / 7 = 14 remainder 2.
        chip.write(0x20, 100);
        chip.write(0x26, 7);
        chip.alu_divide();
        assert_eq!(
            (chip.r4828, chip.r4829, chip.r482a, chip.r482b),
            (14, 0, 0, 0)
        );
        assert_eq!((chip.r482c, chip.r482d), (2, 0));

        // Division by zero yields quotient 0 and remainder = dividend.
        chip.write(0x20, 42);
        chip.write(0x26, 0);
        chip.alu_divide();
        assert_eq!(chip.r4828, 0);
        assert_eq!((chip.r482c, chip.r482d), (42, 0));
    }

    #[test]
    fn pending_units_run_on_the_next_tick() {
        let mut chip = make_chip();
        // Writing $4825 schedules a multiply and stalls 30 clocks.
        chip.write(0x20, 6);
        chip.write(0x24, 7);
        chip.write(0x25, 0);
        assert!(chip.mul_pending);
        let stall = chip.run(1);
        assert_eq!(stall, 30);
        assert!(!chip.mul_pending);
        assert_eq!(chip.r4828, 42);
        // Nothing pending: no stall.
        assert_eq!(chip.run(10), 0);

        // A DCU trigger stalls 20 clocks.
        chip.write(0x06, 0);
        assert!(chip.dcu_pending);
        assert_eq!(chip.run(1), 20);
        assert_eq!(chip.read(0x0C, 0) & 0x80, 0x80);
    }

    #[test]
    fn data_port_reads_and_increments() {
        let mut chip = make_chip();
        // Point at data ROM offset 0x100 with no adjust/stride.
        chip.write(0x11, 0x00);
        chip.write(0x12, 0x01);
        chip.write(0x13, 0x00);
        assert_eq!(chip.read(0x10, 0), 1);
        // Bit 0 of $4818 selects the stride register; the default is 1.
        assert_eq!((chip.r4812, chip.r4813), (0x01, 0x00));
        // A second read advances by one again.
        assert_eq!(chip.r4812, 0x01);
    }

    #[test]
    fn decompressor_bit_helpers() {
        // Inverse Morton transform, golden values from the ares algorithm.
        assert_eq!(Spc7110Decompressor::deinterleave(0x1, 8), 0x10);
        assert_eq!(Spc7110Decompressor::deinterleave(0x2, 8), 0x01);
        assert_eq!(Spc7110Decompressor::deinterleave(0x3, 8), 0x11);
        assert_eq!(Spc7110Decompressor::deinterleave(0x8001, 16), 0x180);
        assert_eq!(Spc7110Decompressor::deinterleave(0x1234, 16), 0x4614);
        assert_eq!(Spc7110Decompressor::deinterleave(0xffff, 16), 0xffff);
        assert_eq!(Spc7110Decompressor::deinterleave(0x0, 16), 0x0);

        // move_to_front moves the matching nibble to slot 0 and is idempotent.
        let list = 0xfedc_ba98_7654_3210u64;
        let moved = Spc7110Decompressor::move_to_front(list, 0x5);
        assert_eq!(moved & 15, 0x5);
        assert_eq!(moved, 0xfedc_ba98_7643_2105);
        // A nibble already at the front leaves the list unchanged.
        assert_eq!(Spc7110Decompressor::move_to_front(moved, 0x5), moved);
    }

    #[test]
    fn rtc_ports_are_only_present_on_f9_boards() {
        assert_eq!(Spc7110::register_offset(0x00_4840), Some(0x40));
        assert_eq!(Spc7110::register_offset(0x80_4841), Some(0x41));
        assert_eq!(Spc7110::register_offset(0x00_4842), Some(0x42));
        assert_eq!(Spc7110::register_offset(0x00_4843), None);

        // No RTC: the ports read as open bus and writes are dropped.
        let mut plain = make_chip();
        assert!(plain.rtc.is_none());
        assert_eq!(plain.read(0x40, 0x5A), 0x5A);
        plain.write(0x41, 0x03);

        // With an RTC the serial protocol works through the same window.
        let rom = vec![0u8; PROM_SIZE];
        let mut chip = Spc7110::new(&rom, RAM_SIZE, true);
        assert!(chip.rtc.is_some());
        chip.write(0x40, 0x01); // chip select
        assert_eq!(chip.read(0x40, 0), 0x01);
        chip.write(0x41, 0x0C); // read command
        chip.write(0x41, 0x00); // start at register 0
                                // The port needs eight oscillator ticks before it is ready again.
        for _ in 0..8 {
            chip.rtc.as_mut().expect("rtc").tick();
        }
        assert_eq!(chip.read(0x41, 0), 0x00); // secondlo
    }

    #[test]
    fn header_detection_matches_mia() {
        assert!(Spc7110::header_has_spc7110(0xf5, 0x3a));
        assert!(Spc7110::header_has_spc7110(0x00, 0x2a));
        assert!(Spc7110::header_has_spc7110(0xf9, 0x00));
        assert!(!Spc7110::header_has_spc7110(0x00, 0x20));
    }
}
