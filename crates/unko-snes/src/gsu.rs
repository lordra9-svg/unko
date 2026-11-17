//! SuperFX (GSU-1/GSU-2) coprocessor.
//!
//! A faithful port of the ares `GSU` interpreter and the `SuperFX` bus wrapper.
//! The GSU is a 16-bit RISC core with 16 general registers, a 512-byte
//! instruction cache, a 32 KiB work RAM and a "plot" pipeline that packs pixels
//! into Game Pak RAM tiles. The S-CPU talks to it through `$3000-$34FF`
//! (registers, cache and a ROM/RAM buffer handshake).
//!
//! Timing is counted in S-CPU master clocks; the GSU oscillator on the boards
//! this core supports runs at the same rate as the S-CPU. The S-CPU advances
//! the GSU with a credit, mirroring our APU/PPU model instead of ares' OS
//! threads.

/// GSU status-flag register bits (`SFR`).
const SFR_Z: u16 = 1 << 1;
const SFR_CY: u16 = 1 << 2;
const SFR_S: u16 = 1 << 3;
const SFR_OV: u16 = 1 << 4;
const SFR_G: u16 = 1 << 5;
const SFR_R: u16 = 1 << 6;
const SFR_ALT1: u16 = 1 << 8;
const SFR_ALT2: u16 = 1 << 9;
const SFR_B: u16 = 1 << 12;
const SFR_IRQ: u16 = 1 << 15;

/// `SCMR` screen-mode register bits.
const SCMR_RON: u8 = 0x10;
const SCMR_RAN: u8 = 0x08;

/// `POR` plot-option bits.
const POR_OBJ: u8 = 0x10;
const POR_FREEZE_HIGH: u8 = 0x08;
const POR_HIGH_NIBBLE: u8 = 0x04;
const POR_DITHER: u8 = 0x02;
const POR_TRANSPARENT: u8 = 0x01;

/// `CFGR` config bits.
const CFGR_IRQ: u8 = 0x80;
const CFGR_MS0: u8 = 0x20;

/// One of the GSU's 16 general-purpose registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GsuRegister {
    /// 16-bit register value.
    pub data: u16,
    /// Set whenever an instruction writes the register.
    pub modified: bool,
}

/// GSU register file.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GsuRegisters {
    /// Next opcode byte prefetched by [`Gsu::peek_pipe`]/[`Gsu::pipe`].
    pub pipeline: u8,
    /// RAM address latched by the last load/store.
    pub ramaddr: u16,
    /// General-purpose registers.
    pub r: [GsuRegister; 16],
    /// Status flags (`SFR`).
    pub sfr: u16,
    /// Program bank.
    pub pbr: u8,
    /// Game Pak ROM bank for the ROM buffer.
    pub rombr: u8,
    /// Game Pak RAM bank for the RAM buffer.
    pub rambr: bool,
    /// Instruction-cache base.
    pub cbr: u16,
    /// Screen base (plot pipeline).
    pub scbr: u8,
    /// Screen mode (`SCMR` raw bits).
    pub scmr: u8,
    /// Current colour (`COLR`).
    pub colr: u8,
    /// Plot options (`POR` raw bits).
    pub por: u8,
    /// Backup-RAM enable.
    pub bramr: bool,
    /// Version code.
    pub vcr: u8,
    /// Config register (`CFGR` raw bits).
    pub cfgr: u8,
    /// Fast-clock select.
    pub clsr: bool,
    /// Clocks until `romdr` is valid.
    pub romcl: u32,
    /// ROM buffer data.
    pub romdr: u8,
    /// Clocks until the pending RAM write completes.
    pub ramcl: u32,
    /// RAM buffer address.
    pub ramar: u16,
    /// RAM buffer data.
    pub ramdr: u8,
    /// Source register index (`from`).
    pub sreg: u32,
    /// Destination register index (`to`).
    pub dreg: u32,
}

/// One 16-byte instruction-cache line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GsuCacheLine {
    /// Line contents.
    #[serde(with = "serde_big_array::BigArray")]
    pub buffer: [u8; 16],
    /// Whether the line is valid.
    pub valid: bool,
}

/// The GSU pixel-packing buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GsuPixelCache {
    /// Tile-memory word offset of the pending write.
    pub offset: u16,
    /// Bit mask of pixels already written.
    pub bitpend: u8,
    /// Per-pixel colour values (bits 0-7 of each byte).
    pub data: [u8; 8],
}

/// The SuperFX coprocessor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Gsu {
    /// Register file.
    pub regs: GsuRegisters,
    /// Instruction cache: 512 bytes in 32 valid lines.
    #[serde(with = "serde_big_array::BigArray")]
    pub cache: [u8; 512],
    /// Instruction-cache line validity.
    pub cache_valid: [bool; 32],
    /// Pixel-pack pipeline (two stages).
    pub pixelcache: [GsuPixelCache; 2],
    /// 32 KiB work RAM.
    pub ram: Vec<u8>,
    /// Whether the GSU holds the S-CPU IRQ line (ares `SuperFX::stop`).
    pub irq_line: bool,
    /// Running master-clock cost of the instruction currently executing.
    ///
    /// Transient (reset at the start of every instruction) but serialized so a
    /// save-state round trip is exact.
    pub cost: u32,
}

impl Default for Gsu {
    fn default() -> Self {
        Self {
            regs: GsuRegisters::default(),
            cache: [0; 512],
            cache_valid: [false; 32],
            pixelcache: [GsuPixelCache::default(); 2],
            ram: Vec::new(),
            irq_line: false,
            cost: 0,
        }
    }
}

impl Gsu {
    /// Create a powered-on GSU with a mask for the supplied ROM size.
    pub fn new() -> Self {
        let mut gsu = Self {
            ram: vec![0; 0x8000],
            ..Self::default()
        };
        gsu.power();
        gsu
    }

    /// Reset the register file (ares `GSU::power`).
    pub fn power(&mut self) {
        self.regs = GsuRegisters::default();
        self.regs.vcr = 0x04;
        self.regs.pipeline = 0x01; // nop
        self.cost = 0;
        for slot in &mut self.pixelcache {
            slot.offset = u16::MAX;
            slot.bitpend = 0;
        }
        self.cache.fill(0);
        self.cache_valid.fill(false);
        self.irq_line = false;
    }

    /// ROM mask used by the GSU bus (power-of-two rounded, ares `romSizeRound`).
    fn rom_mask(rom: &[u8]) -> u32 {
        let mut size = rom.len() as u32;
        if size == 0 {
            return 0;
        }
        if size & (size - 1) == 0 {
            return size - 1;
        }
        let mut count = 0;
        while size != 0 {
            size >>= 1;
            count += 1;
        }
        (1u32 << count) - 1
    }

    fn ram_mask(&self) -> u32 {
        (self.ram.len() as u32).wrapping_sub(1)
    }

    // --- SFR flag helpers -------------------------------------------------

    /// Raw status register as seen by the S-CPU (ares masks undefined bits).
    pub const fn sfr(&self) -> u16 {
        self.regs.sfr & 0x9F7E
    }

    fn f(&self, bit: u16) -> bool {
        self.regs.sfr & bit != 0
    }

    fn set_f(&mut self, bit: u16, value: bool) {
        if value {
            self.regs.sfr |= bit;
        } else {
            self.regs.sfr &= !bit;
        }
    }

    fn sfr_b(&self) -> bool {
        self.f(SFR_B)
    }
    fn sfr_alt1(&self) -> bool {
        self.f(SFR_ALT1)
    }
    fn sfr_alt2(&self) -> bool {
        self.f(SFR_ALT2)
    }
    fn sfr_cy(&self) -> bool {
        self.f(SFR_CY)
    }
    fn sfr_s(&self) -> bool {
        self.f(SFR_S)
    }
    fn sfr_z(&self) -> bool {
        self.f(SFR_Z)
    }
    fn sfr_ov(&self) -> bool {
        self.f(SFR_OV)
    }
    fn sfr_g(&self) -> bool {
        self.f(SFR_G)
    }
    fn set_sfr_b(&mut self, v: bool) {
        self.set_f(SFR_B, v);
    }
    fn set_sfr_s(&mut self, v: bool) {
        self.set_f(SFR_S, v);
    }
    fn set_sfr_z(&mut self, v: bool) {
        self.set_f(SFR_Z, v);
    }
    fn set_sfr_cy(&mut self, v: bool) {
        self.set_f(SFR_CY, v);
    }
    fn set_sfr_ov(&mut self, v: bool) {
        self.set_f(SFR_OV, v);
    }
    fn set_sfr_g(&mut self, v: bool) {
        self.set_f(SFR_G, v);
    }

    fn por_obj(&self) -> bool {
        self.regs.por & POR_OBJ != 0
    }
    fn por_freeze_high(&self) -> bool {
        self.regs.por & POR_FREEZE_HIGH != 0
    }
    fn por_high_nibble(&self) -> bool {
        self.regs.por & POR_HIGH_NIBBLE != 0
    }
    fn por_dither(&self) -> bool {
        self.regs.por & POR_DITHER != 0
    }
    fn por_transparent(&self) -> bool {
        self.regs.por & POR_TRANSPARENT != 0
    }
    fn scmr_md(&self) -> u8 {
        self.regs.scmr & 0x03
    }
    fn scmr_ht(&self) -> u8 {
        ((self.regs.scmr >> 5) & 1) << 1 | ((self.regs.scmr >> 2) & 1)
    }
    fn cfgr_ms0(&self) -> bool {
        self.regs.cfgr & CFGR_MS0 != 0
    }

    /// Whether the GSU is currently executing (`SFR.G`).
    pub const fn running(&self) -> bool {
        self.regs.sfr & SFR_G != 0
    }

    /// Whether the GSU has ROM access enabled (`SCMR.RON`).
    pub const fn rom_enabled(&self) -> bool {
        self.regs.scmr & SCMR_RON != 0
    }

    /// Whether the GSU has RAM access enabled (`SCMR.RAN`).
    pub const fn ram_enabled(&self) -> bool {
        self.regs.scmr & SCMR_RAN != 0
    }

    fn source(&self) -> u16 {
        self.regs.r[self.regs.sreg as usize & 15].data
    }

    fn dest_set(&mut self, value: u16) {
        let index = self.regs.dreg as usize & 15;
        self.regs.r[index].data = value;
        self.regs.r[index].modified = true;
    }

    fn reset_latch(&mut self) {
        self.set_sfr_b(false);
        self.set_f(SFR_ALT1, false);
        self.set_f(SFR_ALT2, false);
        self.regs.sreg = 0;
        self.regs.dreg = 0;
    }

    // --- Memory -----------------------------------------------------------

    /// Read one byte from the GSU's own address space.
    pub fn read(&self, rom: &[u8], address: u32) -> u8 {
        if address & 0xC0_0000 == 0 {
            // $00-3f: LoROM-style Game Pak ROM.
            let offset = (((address & 0x3F_0000) >> 1) | (address & 0x7FFF)) & Self::rom_mask(rom);
            return rom.get(offset as usize).copied().unwrap_or(0);
        }
        if address & 0xE0_0000 == 0x40_0000 {
            // $40-5f: linear Game Pak ROM.
            let offset = address & Self::rom_mask(rom);
            return rom.get(offset as usize).copied().unwrap_or(0);
        }
        if address & 0xFE_0000 == 0x70_0000 {
            // $70-71: Game Pak RAM.
            let offset = address & self.ram_mask();
            return self.ram.get(offset as usize).copied().unwrap_or(0);
        }
        0
    }

    /// Write one byte into the GSU's own address space.
    pub fn write(&mut self, _rom: &[u8], address: u32, value: u8) {
        if address & 0xFE_0000 == 0x70_0000 {
            let offset = (address & self.ram_mask()) as usize;
            if let Some(byte) = self.ram.get_mut(offset) {
                *byte = value;
            }
        }
    }

    /// Read the next opcode byte, filling the instruction cache if needed.
    fn read_opcode(&mut self, rom: &[u8], address: u16) -> u8 {
        let offset = address.wrapping_sub(self.regs.cbr);
        if offset < 512 {
            let line = usize::from(offset >> 4);
            if !self.cache_valid[line] {
                let dp = usize::from(offset & 0xFFF0);
                let sp = (u32::from(self.regs.pbr) << 16)
                    | u32::from((self.regs.cbr.wrapping_add(dp as u16)) & 0xFFF0);
                for n in 0..16 {
                    self.step(rom, if self.regs.clsr { 5 } else { 6 });
                    self.cache[dp + n] = self.read(rom, sp.wrapping_add(n as u32));
                }
                self.cache_valid[line] = true;
            } else {
                self.step(rom, if self.regs.clsr { 1 } else { 2 });
            }
            return self.cache[usize::from(offset)];
        }

        if self.regs.pbr <= 0x5F {
            self.sync_rom_buffer(rom);
        } else {
            self.sync_ram_buffer(rom);
        }
        self.step(rom, if self.regs.clsr { 5 } else { 6 });
        self.read(rom, (u32::from(self.regs.pbr) << 16) | u32::from(address))
    }

    fn peek_pipe(&mut self, rom: &[u8]) -> u8 {
        let result = self.regs.pipeline;
        self.regs.pipeline = self.read_opcode(rom, self.regs.r[15].data);
        self.regs.r[15].modified = false;
        result
    }

    fn pipe(&mut self, rom: &[u8]) -> u8 {
        let result = self.regs.pipeline;
        self.regs.r[15].data = self.regs.r[15].data.wrapping_add(1);
        self.regs.pipeline = self.read_opcode(rom, self.regs.r[15].data);
        self.regs.r[15].modified = false;
        result
    }

    fn flush_cache(&mut self) {
        self.cache_valid.fill(false);
    }

    fn read_cache(&self, address: u16) -> u8 {
        let address = (address.wrapping_add(self.regs.cbr)) & 511;
        self.cache[usize::from(address)]
    }

    fn write_cache(&mut self, address: u16, data: u8) {
        let address = (address.wrapping_add(self.regs.cbr)) & 511;
        self.cache[usize::from(address)] = data;
        if address & 15 == 15 {
            self.cache_valid[usize::from(address >> 4)] = true;
        }
    }

    // --- Timing and buffers ----------------------------------------------

    /// Account `clocks` of GSU time.
    fn step(&mut self, rom: &[u8], clocks: u32) {
        self.cost += clocks;
        if self.regs.romcl != 0 {
            let used = clocks.min(self.regs.romcl);
            self.regs.romcl -= used;
            if self.regs.romcl == 0 {
                self.set_f(SFR_R, false);
                let address = (u32::from(self.regs.rombr) << 16) | u32::from(self.regs.r[14].data);
                self.regs.romdr = self.read(rom, address);
            }
        }
        if self.regs.ramcl != 0 {
            let used = clocks.min(self.regs.ramcl);
            self.regs.ramcl -= used;
            if self.regs.ramcl == 0 {
                let address =
                    0x70_0000 | ((u32::from(self.regs.rambr)) << 16) | u32::from(self.regs.ramar);
                let data = self.regs.ramdr;
                self.write(rom, address, data);
            }
        }
    }

    fn sync_rom_buffer(&mut self, rom: &[u8]) {
        if self.regs.romcl != 0 {
            let clocks = self.regs.romcl;
            self.step(rom, clocks);
        }
    }

    fn read_rom_buffer(&mut self, rom: &[u8]) -> u8 {
        self.sync_rom_buffer(rom);
        self.regs.romdr
    }

    fn update_rom_buffer(&mut self) {
        self.set_f(SFR_R, true);
        self.regs.romcl = if self.regs.clsr { 5 } else { 6 };
    }

    fn sync_ram_buffer(&mut self, rom: &[u8]) {
        if self.regs.ramcl != 0 {
            let clocks = self.regs.ramcl;
            self.step(rom, clocks);
        }
    }

    fn read_ram_buffer(&mut self, rom: &[u8], address: u16) -> u8 {
        self.sync_ram_buffer(rom);
        let full = 0x70_0000 | (u32::from(self.regs.rambr) << 16) | u32::from(address);
        self.read(rom, full)
    }

    fn write_ram_buffer(&mut self, rom: &[u8], address: u16, data: u8) {
        self.sync_ram_buffer(rom);
        self.regs.ramcl = if self.regs.clsr { 5 } else { 6 };
        self.regs.ramar = address;
        self.regs.ramdr = data;
    }

    /// Flush any pending ROM/RAM buffer operation (the S-CPU synchronizing).
    pub fn synchronize(&mut self, rom: &[u8]) {
        self.sync_rom_buffer(rom);
        self.sync_ram_buffer(rom);
    }

    /// Run one GSU step and return the master clocks it consumed.
    pub fn run_one(&mut self, rom: &[u8]) -> u32 {
        self.cost = 0;
        if self.sfr_g() {
            let opcode = self.peek_pipe(rom);
            self.instruction(opcode, rom);
            if self.regs.r[14].modified {
                self.regs.r[14].modified = false;
                self.update_rom_buffer();
            }
            if self.regs.r[15].modified {
                self.regs.r[15].modified = false;
            } else {
                self.regs.r[15].data = self.regs.r[15].data.wrapping_add(1);
            }
        } else {
            self.step(rom, 6);
        }
        self.cost
    }

    // --- S-CPU interface ($3000-$34FF) ------------------------------------

    /// Read a GSU register or the instruction cache (S-CPU side).
    pub fn read_io(&mut self, address: u32) -> u8 {
        let address = 0x3000 | (address & 0x3FF);
        if (0x3100..=0x32FF).contains(&address) {
            return self.read_cache((address - 0x3100) as u16);
        }
        if (0x3000..=0x301F).contains(&address) {
            let index = ((address >> 1) & 15) as usize;
            return if address & 1 == 0 {
                self.regs.r[index].data as u8
            } else {
                (self.regs.r[index].data >> 8) as u8
            };
        }
        match address {
            0x3030 => self.sfr() as u8,
            0x3031 => {
                let high = (self.sfr() >> 8) as u8;
                self.set_f(SFR_IRQ, false);
                self.irq_line = false;
                high
            }
            0x3034 => self.regs.pbr,
            0x3036 => self.regs.rombr,
            0x303B => self.regs.vcr,
            0x303C => u8::from(self.regs.rambr),
            0x303E => self.regs.cbr as u8,
            0x303F => (self.regs.cbr >> 8) as u8,
            _ => 0,
        }
    }

    /// Write a GSU register or the instruction cache (S-CPU side).
    pub fn write_io(&mut self, address: u32, data: u8) {
        let address = 0x3000 | (address & 0x3FF);
        if (0x3100..=0x32FF).contains(&address) {
            self.write_cache((address - 0x3100) as u16, data);
            return;
        }
        if (0x3000..=0x301F).contains(&address) {
            let index = ((address >> 1) & 15) as usize;
            if address & 1 == 0 {
                self.regs.r[index].data = (self.regs.r[index].data & 0xFF00) | u16::from(data);
            } else {
                self.regs.r[index].data =
                    (u16::from(data) << 8) | (self.regs.r[index].data & 0x00FF);
            }
            self.regs.r[index].modified = true;
            if index == 14 {
                self.update_rom_buffer();
            }
            if address == 0x301F {
                self.set_sfr_g(true);
            }
            return;
        }
        match address {
            0x3030 => {
                let was_running = self.sfr_g();
                self.regs.sfr = (self.regs.sfr & 0xFF00) | u16::from(data);
                if was_running && !self.sfr_g() {
                    self.regs.cbr = 0;
                    self.flush_cache();
                }
            }
            0x3031 => self.regs.sfr = (u16::from(data) << 8) | (self.regs.sfr & 0x00FF),
            0x3033 => self.regs.bramr = data & 1 != 0,
            0x3034 => {
                self.regs.pbr = data & 0x7F;
                self.flush_cache();
            }
            0x3037 => self.regs.cfgr = data,
            0x3038 => self.regs.scbr = data,
            0x3039 => self.regs.clsr = data & 1 != 0,
            0x303A => self.regs.scmr = data,
            _ => {}
        }
    }

    /// Read or write the 32 KiB work RAM from the S-CPU.
    pub fn read_ram(&self, offset: u32) -> u8 {
        let offset = (offset & self.ram_mask()) as usize;
        self.ram.get(offset).copied().unwrap_or(0)
    }

    /// Write the 32 KiB work RAM from the S-CPU.
    pub fn write_ram(&mut self, offset: u32, value: u8) {
        let offset = (offset & self.ram_mask()) as usize;
        if let Some(byte) = self.ram.get_mut(offset) {
            *byte = value;
        }
    }

    /// Take and clear a pending S-CPU IRQ request.
    pub fn take_irq(&mut self) -> bool {
        let pending = self.irq_line;
        self.irq_line = false;
        pending
    }

    // --- Instruction interpreter -----------------------------------------

    fn instruction(&mut self, opcode: u8, rom: &[u8]) {
        let n = opcode & 15;
        match opcode {
            0x00 => self.op_stop(),
            0x01 => self.reset_latch(),
            0x02 => self.op_cache(),
            0x03 => self.op_lsr(),
            0x04 => self.op_rol(),
            0x05 => self.op_branch(rom, true),
            0x06 => self.op_branch(rom, !(self.sfr_s() ^ self.sfr_ov())),
            0x07 => self.op_branch(rom, self.sfr_s() ^ self.sfr_ov()),
            0x08 => self.op_branch(rom, !self.sfr_z()),
            0x09 => self.op_branch(rom, self.sfr_z()),
            0x0A => self.op_branch(rom, !self.sfr_s()),
            0x0B => self.op_branch(rom, self.sfr_s()),
            0x0C => self.op_branch(rom, !self.sfr_cy()),
            0x0D => self.op_branch(rom, self.sfr_cy()),
            0x0E => self.op_branch(rom, !self.sfr_ov()),
            0x0F => self.op_branch(rom, self.sfr_ov()),
            0x10..=0x1F => self.op_to_move(n),
            0x20..=0x2F => self.op_with(n),
            0x30..=0x3B => self.op_store(rom, n),
            0x3C => self.op_loop(),
            0x3D => {
                self.set_sfr_b(false);
                self.set_f(SFR_ALT1, true);
            }
            0x3E => {
                self.set_sfr_b(false);
                self.set_f(SFR_ALT2, true);
            }
            0x3F => {
                self.set_sfr_b(false);
                self.set_f(SFR_ALT1, true);
                self.set_f(SFR_ALT2, true);
            }
            0x40..=0x4B => self.op_load(rom, n),
            0x4C => self.op_plot_rpix(rom),
            0x4D => self.op_swap(),
            0x4E => self.op_color_cmode(),
            0x4F => self.op_not(),
            0x50..=0x5F => self.op_add_adc(n),
            0x60..=0x6F => self.op_sub_sbc_cmp(n),
            0x70 => self.op_merge(),
            0x71..=0x7F => self.op_and_bic(n),
            0x80..=0x8F => self.op_mult_umult(rom, n),
            0x90 => self.op_sbk(rom),
            0x91..=0x94 => self.op_link(n),
            0x95 => self.op_sex(),
            0x96 => self.op_asr_div2(),
            0x97 => self.op_ror(),
            0x98..=0x9D => self.op_jmp_ljmp(n),
            0x9E => self.op_lob(),
            0x9F => self.op_fmult_lmult(rom),
            0xA0..=0xAF => self.op_ibt_lms_sms(rom, n),
            0xB0..=0xBF => self.op_from_moves(n),
            0xC0 => self.op_hib(),
            0xC1..=0xCF => self.op_or_xor(n),
            0xD0..=0xDE => self.op_inc(n),
            0xDF => self.op_getc_ramb_romb(rom),
            0xE0..=0xEE => self.op_dec(n),
            0xEF => self.op_getb(rom),
            0xF0..=0xFF => self.op_iwt_lm_sm(rom, n),
        }
    }

    fn op_stop(&mut self) {
        if self.regs.cfgr & CFGR_IRQ == 0 {
            self.set_f(SFR_IRQ, true);
            self.irq_line = true;
        }
        self.set_sfr_g(false);
        self.regs.pipeline = 0x01;
        self.reset_latch();
    }

    fn op_cache(&mut self) {
        let base = self.regs.r[15].data & 0xFFF0;
        if self.regs.cbr != base {
            self.regs.cbr = base;
            self.flush_cache();
        }
        self.reset_latch();
    }

    fn op_lsr(&mut self) {
        let source = self.source();
        self.set_sfr_cy(source & 1 != 0);
        let value = source >> 1;
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_rol(&mut self) {
        let source = self.source();
        let carry = source & 0x8000 != 0;
        let value = (source << 1) | u16::from(self.sfr_cy());
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_cy(carry);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_branch(&mut self, rom: &[u8], take: bool) {
        let displacement = self.pipe(rom) as i8;
        if take {
            self.regs.r[15].data = self.regs.r[15].data.wrapping_add(displacement as u16);
            self.regs.r[15].modified = true;
        }
    }

    fn op_to_move(&mut self, n: u8) {
        if !self.sfr_b() {
            self.regs.dreg = u32::from(n);
        } else {
            let value = self.source();
            self.regs.r[n as usize & 15].data = value;
            self.regs.r[n as usize & 15].modified = true;
            self.reset_latch();
        }
    }

    fn op_with(&mut self, n: u8) {
        self.regs.sreg = u32::from(n);
        self.regs.dreg = u32::from(n);
        self.set_sfr_b(true);
    }

    fn op_store(&mut self, rom: &[u8], n: u8) {
        self.regs.ramaddr = self.regs.r[n as usize & 15].data;
        let value = self.source();
        self.write_ram_buffer(rom, self.regs.ramaddr, value as u8);
        if !self.sfr_alt1() {
            self.write_ram_buffer(rom, self.regs.ramaddr ^ 1, (value >> 8) as u8);
        }
        self.reset_latch();
    }

    fn op_loop(&mut self) {
        self.regs.r[12].data = self.regs.r[12].data.wrapping_sub(1);
        self.regs.r[12].modified = true;
        let value = self.regs.r[12].data;
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        if !self.sfr_z() {
            self.regs.r[15].data = self.regs.r[13].data;
            self.regs.r[15].modified = true;
        }
        self.reset_latch();
    }

    fn op_load(&mut self, rom: &[u8], n: u8) {
        self.regs.ramaddr = self.regs.r[n as usize & 15].data;
        let low = self.read_ram_buffer(rom, self.regs.ramaddr);
        let mut value = u16::from(low);
        if !self.sfr_alt1() {
            value |= u16::from(self.read_ram_buffer(rom, self.regs.ramaddr ^ 1)) << 8;
        }
        self.dest_set(value);
        self.reset_latch();
    }

    fn op_plot_rpix(&mut self, rom: &[u8]) {
        if !self.sfr_alt1() {
            let x = self.regs.r[1].data as u8;
            let y = self.regs.r[2].data as u8;
            self.plot(rom, x, y);
            self.regs.r[1].data = self.regs.r[1].data.wrapping_add(1);
            self.regs.r[1].modified = true;
        } else {
            let x = self.regs.r[1].data as u8;
            let y = self.regs.r[2].data as u8;
            let value = u16::from(self.rpix(rom, x, y));
            self.dest_set(value);
            self.set_sfr_s(value & 0x8000 != 0);
            self.set_sfr_z(value == 0);
        }
        self.reset_latch();
    }

    fn op_swap(&mut self) {
        let source = self.source();
        let value = source.rotate_left(8);
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_color_cmode(&mut self) {
        let source = self.source();
        if !self.sfr_alt1() {
            self.regs.colr = self.color(source as u8);
        } else {
            self.regs.por = source as u8;
        }
        self.reset_latch();
    }

    fn op_not(&mut self) {
        let value = !self.source();
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_add_adc(&mut self, n: u8) {
        let source = self.source();
        let operand = if self.sfr_alt2() {
            u16::from(n)
        } else {
            self.regs.r[n as usize & 15].data
        };
        let carry = u32::from(self.sfr_alt1() && self.sfr_cy());
        let result = i32::from(source) + i32::from(operand) + carry as i32;
        self.set_sfr_ov((!(source ^ operand)) & (operand ^ (result as u16)) & 0x8000 != 0);
        self.set_sfr_s(result & 0x8000 != 0);
        self.set_sfr_cy(result >= 0x10000);
        self.set_sfr_z(result as u16 == 0);
        self.dest_set(result as u16);
        self.reset_latch();
    }

    fn op_sub_sbc_cmp(&mut self, n: u8) {
        let source = self.source();
        let operand = if !self.sfr_alt2() || self.sfr_alt1() {
            self.regs.r[n as usize & 15].data
        } else {
            u16::from(n)
        };
        let borrow = u32::from(!self.sfr_alt2() && self.sfr_alt1() && !self.sfr_cy());
        let result = i32::from(source) - i32::from(operand) - borrow as i32;
        self.set_sfr_ov((source ^ operand) & (source ^ (result as u16)) & 0x8000 != 0);
        self.set_sfr_s(result & 0x8000 != 0);
        self.set_sfr_cy(result >= 0);
        self.set_sfr_z(result as u16 == 0);
        if !self.sfr_alt2() || !self.sfr_alt1() {
            self.dest_set(result as u16);
        }
        self.reset_latch();
    }

    fn op_merge(&mut self) {
        let value = (self.regs.r[7].data & 0xFF00) | (self.regs.r[8].data >> 8);
        self.dest_set(value);
        self.set_sfr_ov(value & 0xC0C0 != 0);
        self.set_sfr_s(value & 0x8080 != 0);
        self.set_sfr_cy(value & 0xE0E0 != 0);
        self.set_sfr_z(value & 0xF0F0 != 0);
        self.reset_latch();
    }

    fn op_and_bic(&mut self, n: u8) {
        let source = self.source();
        let operand = if self.sfr_alt2() {
            u16::from(n)
        } else {
            self.regs.r[n as usize & 15].data
        };
        let value = source & if self.sfr_alt1() { !operand } else { operand };
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_mult_umult(&mut self, rom: &[u8], n: u8) {
        let source = self.source();
        let operand = if self.sfr_alt2() {
            u16::from(n)
        } else {
            self.regs.r[n as usize & 15].data
        };
        let value = if self.sfr_alt1() {
            u16::from(source as u8) * u16::from(operand as u8)
        } else {
            ((source as i8 as i16) * (operand as i8 as i16)) as u16
        };
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
        if !self.cfgr_ms0() {
            self.step(rom, if self.regs.clsr { 1 } else { 2 });
        }
    }

    fn op_sbk(&mut self, rom: &[u8]) {
        let value = self.source();
        self.write_ram_buffer(rom, self.regs.ramaddr, value as u8);
        self.write_ram_buffer(rom, self.regs.ramaddr ^ 1, (value >> 8) as u8);
        self.reset_latch();
    }

    fn op_link(&mut self, n: u8) {
        self.regs.r[11].data = self.regs.r[15].data.wrapping_add(u16::from(n));
        self.regs.r[11].modified = true;
        self.reset_latch();
    }

    fn op_sex(&mut self) {
        let value = (self.source() as i8) as i16 as u16;
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_asr_div2(&mut self) {
        let source = self.source();
        self.set_sfr_cy(source & 1 != 0);
        let mut value = ((source as i16) >> 1) as u16;
        if self.sfr_alt1() {
            value = value.wrapping_add(((u32::from(source) + 1) >> 16) as u16);
        }
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_ror(&mut self) {
        let source = self.source();
        let carry = source & 1 != 0;
        let value = (u16::from(self.sfr_cy()) << 15) | (source >> 1);
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_cy(carry);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_jmp_ljmp(&mut self, n: u8) {
        let target = self.regs.r[n as usize & 15].data;
        if !self.sfr_alt1() {
            self.regs.r[15].data = target;
            self.regs.r[15].modified = true;
        } else {
            self.regs.pbr = (target & 0x7F) as u8;
            self.regs.r[15].data = self.source();
            self.regs.r[15].modified = true;
            self.regs.cbr = self.regs.r[15].data & 0xFFF0;
            self.flush_cache();
        }
        self.reset_latch();
    }

    fn op_lob(&mut self) {
        let value = self.source() & 0xFF;
        self.dest_set(value);
        self.set_sfr_s(value & 0x80 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_fmult_lmult(&mut self, rom: &[u8]) {
        let result = i32::from(self.source() as i16) * i32::from(self.regs.r[6].data as i16);
        if self.sfr_alt1() {
            self.regs.r[4].data = result as u32 as u16;
            self.regs.r[4].modified = true;
        }
        let value = (result >> 16) as u16;
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_cy(result & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
        let clocks = if self.cfgr_ms0() { 3 } else { 7 };
        self.step(rom, clocks * if self.regs.clsr { 1 } else { 2 });
    }

    fn op_ibt_lms_sms(&mut self, rom: &[u8], n: u8) {
        if self.sfr_alt1() {
            let address = u16::from(self.pipe(rom)) << 1;
            self.regs.ramaddr = address;
            let low = self.read_ram_buffer(rom, address);
            let high = self.read_ram_buffer(rom, address ^ 1);
            let value = (u16::from(high) << 8) | u16::from(low);
            self.regs.r[n as usize & 15].data = value;
            self.regs.r[n as usize & 15].modified = true;
        } else if self.sfr_alt2() {
            let address = u16::from(self.pipe(rom)) << 1;
            self.regs.ramaddr = address;
            let value = self.regs.r[n as usize & 15].data;
            self.write_ram_buffer(rom, address, value as u8);
            self.write_ram_buffer(rom, address ^ 1, (value >> 8) as u8);
        } else {
            let value = (self.pipe(rom) as i8) as i16 as u16;
            self.regs.r[n as usize & 15].data = value;
            self.regs.r[n as usize & 15].modified = true;
        }
        self.reset_latch();
    }

    fn op_from_moves(&mut self, n: u8) {
        if !self.sfr_b() {
            self.regs.sreg = u32::from(n);
        } else {
            let value = self.regs.r[n as usize & 15].data;
            self.dest_set(value);
            self.set_sfr_ov(value & 0x80 != 0);
            self.set_sfr_s(value & 0x8000 != 0);
            self.set_sfr_z(value == 0);
            self.reset_latch();
        }
    }

    fn op_hib(&mut self) {
        let value = self.source() >> 8;
        self.dest_set(value);
        self.set_sfr_s(value & 0x80 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_or_xor(&mut self, n: u8) {
        let source = self.source();
        let operand = if self.sfr_alt2() {
            u16::from(n)
        } else {
            self.regs.r[n as usize & 15].data
        };
        let value = if self.sfr_alt1() {
            source ^ operand
        } else {
            source | operand
        };
        self.dest_set(value);
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_inc(&mut self, n: u8) {
        let index = n as usize & 15;
        self.regs.r[index].data = self.regs.r[index].data.wrapping_add(1);
        self.regs.r[index].modified = true;
        let value = self.regs.r[index].data;
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_getc_ramb_romb(&mut self, rom: &[u8]) {
        if !self.sfr_alt2() {
            let byte = self.read_rom_buffer(rom);
            self.regs.colr = self.color(byte);
        } else if !self.sfr_alt1() {
            self.sync_ram_buffer(rom);
            self.regs.rambr = self.source() & 1 != 0;
        } else {
            self.sync_rom_buffer(rom);
            self.regs.rombr = (self.source() & 0x7F) as u8;
        }
        self.reset_latch();
    }

    fn op_dec(&mut self, n: u8) {
        let index = n as usize & 15;
        self.regs.r[index].data = self.regs.r[index].data.wrapping_sub(1);
        self.regs.r[index].modified = true;
        let value = self.regs.r[index].data;
        self.set_sfr_s(value & 0x8000 != 0);
        self.set_sfr_z(value == 0);
        self.reset_latch();
    }

    fn op_getb(&mut self, rom: &[u8]) {
        let alt = (u8::from(self.sfr_alt2()) << 1) | u8::from(self.sfr_alt1());
        let source = self.source();
        let value = match alt {
            0 => u16::from(self.read_rom_buffer(rom)),
            1 => (u16::from(self.read_rom_buffer(rom)) << 8) | (source & 0x00FF),
            2 => (source & 0xFF00) | u16::from(self.read_rom_buffer(rom)),
            _ => (self.read_rom_buffer(rom) as i8) as i16 as u16,
        };
        self.dest_set(value);
        self.reset_latch();
    }

    fn op_iwt_lm_sm(&mut self, rom: &[u8], n: u8) {
        if self.sfr_alt1() {
            let low = self.pipe(rom);
            let high = self.pipe(rom);
            let address = u16::from(low) | (u16::from(high) << 8);
            self.regs.ramaddr = address;
            let lo = self.read_ram_buffer(rom, address);
            let hi = self.read_ram_buffer(rom, address ^ 1);
            let value = (u16::from(hi) << 8) | u16::from(lo);
            self.regs.r[n as usize & 15].data = value;
            self.regs.r[n as usize & 15].modified = true;
        } else if self.sfr_alt2() {
            let low = self.pipe(rom);
            let high = self.pipe(rom);
            let address = u16::from(low) | (u16::from(high) << 8);
            self.regs.ramaddr = address;
            let value = self.regs.r[n as usize & 15].data;
            self.write_ram_buffer(rom, address, value as u8);
            self.write_ram_buffer(rom, address ^ 1, (value >> 8) as u8);
        } else {
            let low = self.pipe(rom);
            let high = self.pipe(rom);
            let value = (u16::from(high) << 8) | u16::from(low);
            self.regs.r[n as usize & 15].data = value;
            self.regs.r[n as usize & 15].modified = true;
        }
        self.reset_latch();
    }

    // --- Plot pipeline ----------------------------------------------------

    fn color(&self, source: u8) -> u8 {
        if self.por_high_nibble() {
            (self.regs.colr & 0xF0) | (source >> 4)
        } else if self.por_freeze_high() {
            (self.regs.colr & 0xF0) | (source & 0x0F)
        } else {
            source
        }
    }

    fn plot(&mut self, rom: &[u8], x: u8, y: u8) {
        if !self.por_transparent() {
            if self.scmr_md() == 3 {
                if self.por_freeze_high() {
                    if self.regs.colr & 0x0F == 0 {
                        return;
                    }
                } else if self.regs.colr == 0 {
                    return;
                }
            } else if self.regs.colr & 0x0F == 0 {
                return;
            }
        }

        let mut color = self.regs.colr;
        if self.por_dither() && self.scmr_md() != 3 {
            if (x ^ y) & 1 != 0 {
                color >>= 4;
            }
            color &= 0x0F;
        }

        let offset = (u16::from(y) << 5) + u16::from(x >> 3);
        if offset != self.pixelcache[0].offset {
            self.flush_pixel_cache(rom, 1);
            self.pixelcache[1] = self.pixelcache[0];
            self.pixelcache[0].bitpend = 0;
            self.pixelcache[0].offset = offset;
        }

        let bit = (x & 7) ^ 7;
        self.pixelcache[0].data[bit as usize] = color;
        self.pixelcache[0].bitpend |= 1 << bit;
        if self.pixelcache[0].bitpend == 0xFF {
            self.flush_pixel_cache(rom, 1);
            self.pixelcache[1] = self.pixelcache[0];
            self.pixelcache[0].bitpend = 0;
        }
    }

    fn rpix(&mut self, rom: &[u8], x: u8, y: u8) -> u8 {
        self.flush_pixel_cache(rom, 1);
        self.flush_pixel_cache(rom, 0);

        let ch = self.character_number(x, y);
        let bpp = 2u32 << (self.scmr_md() - (self.scmr_md() >> 1));
        let addr = 0x70_0000
            + (ch * (bpp << 3))
            + (u32::from(self.regs.scbr) << 10)
            + (u32::from(y & 0x07) * 2);
        let mut data = 0u8;
        let bit = (x & 7) ^ 7;
        for n in 0..bpp {
            let byte = ((n >> 1) << 4) + (n & 1);
            self.step(rom, if self.regs.clsr { 5 } else { 6 });
            data |= ((self.read(rom, addr + byte) >> bit) & 1) << n;
        }
        data
    }

    fn flush_pixel_cache(&mut self, rom: &[u8], index: usize) {
        let cache = self.pixelcache[index];
        if cache.bitpend == 0 {
            return;
        }

        let x = (cache.offset << 3) as u8;
        let y = (cache.offset >> 5) as u8;
        let ch = self.character_number(x, y);
        let bpp = 2u32 << (self.scmr_md() - (self.scmr_md() >> 1));
        let addr = 0x70_0000
            + (ch * (bpp << 3))
            + (u32::from(self.regs.scbr) << 10)
            + (u32::from(y & 0x07) * 2);

        for n in 0..bpp {
            let byte = ((n >> 1) << 4) + (n & 1);
            let mut data = 0u8;
            for (bit, &value) in cache.data.iter().enumerate() {
                data |= ((value >> n) & 1) << bit;
            }
            if cache.bitpend != 0xFF {
                self.step(rom, if self.regs.clsr { 5 } else { 6 });
                data &= cache.bitpend;
                data |= self.read(rom, addr + byte) & !cache.bitpend;
            }
            self.step(rom, if self.regs.clsr { 5 } else { 6 });
            self.write(rom, addr + byte, data);
        }

        self.pixelcache[index].bitpend = 0;
    }

    fn character_number(&self, x: u8, y: u8) -> u32 {
        let index = if self.por_obj() { 3 } else { self.scmr_ht() };
        match index {
            0 => (u32::from(x & 0xF8) << 1) + (u32::from(y & 0xF8) >> 3),
            1 => {
                (u32::from(x & 0xF8) << 1) + (u32::from(x & 0xF8) >> 1) + (u32::from(y & 0xF8) >> 3)
            }
            2 => (u32::from(x & 0xF8) << 1) + u32::from(x & 0xF8) + (u32::from(y & 0xF8) >> 3),
            _ => {
                (u32::from(y & 0x80) << 2)
                    + (u32::from(x & 0x80) << 1)
                    + (u32::from(y & 0x78) << 1)
                    + (u32::from(x & 0x78) >> 3)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a GSU program placed at the start of a synthetic ROM until it stops.
    fn run(program: &[u8], start: u16) -> Gsu {
        let rom = program.to_vec();
        let mut gsu = Gsu::new();
        gsu.write_io(0x3034, 0x00); // PBR = 0
        gsu.write_io(0x303A, SCMR_RON | SCMR_RAN); // RON | RAN
        gsu.write_io(0x3039, 0x01); // fast clock
        gsu.write_io(0x301E, start as u8);
        gsu.write_io(0x301F, (start >> 8) as u8); // starts the GSU
        assert!(gsu.running());
        for _ in 0..10_000 {
            if !gsu.running() {
                break;
            }
            gsu.run_one(&rom);
        }
        assert!(!gsu.running(), "GSU program did not stop");
        gsu
    }

    #[test]
    fn add_sets_carry_zero_and_overflow() {
        // iwt r1,#$7fff; iwt r0,#$8001; with r1; add r0; stop
        let program = [0xF1, 0xFF, 0x7F, 0xF0, 0x01, 0x80, 0x21, 0x50, 0x00];
        let gsu = run(&program, 0x8000);
        assert_eq!(gsu.regs.r[1].data, 0x0000);
        assert!(gsu.f(SFR_CY));
        assert!(gsu.f(SFR_Z));
        assert!(!gsu.f(SFR_S));
        assert!(!gsu.f(SFR_OV));
    }

    #[test]
    fn unsigned_multiply_uses_a_sixteen_bit_product() {
        // iwt r1,#$00ff; iwt r0,#$00ff; with r1; alt1; umult r0; stop
        let program = [0xF1, 0xFF, 0x00, 0xF0, 0xFF, 0x00, 0x21, 0x3D, 0x80, 0x00];
        let gsu = run(&program, 0x8000);
        assert_eq!(gsu.regs.r[1].data, 0xFE01);
        assert!(gsu.f(SFR_S));
        assert!(!gsu.f(SFR_Z));
    }

    #[test]
    fn signed_multiply_uses_a_sixteen_bit_product() {
        // iwt r1,#$00ff; iwt r0,#$00ff; with r1; mult r0; stop
        let program = [0xF1, 0xFF, 0x00, 0xF0, 0xFF, 0x00, 0x21, 0x80, 0x00];
        let gsu = run(&program, 0x8000);
        assert_eq!(gsu.regs.r[1].data, 0x0001);
        assert!(!gsu.f(SFR_Z));
    }

    #[test]
    fn ram_store_and_load_round_trip_through_the_buffer() {
        // iwt r7,#$0010; iwt r0,#$1234; stw (r7); ldw (r7); stop
        //
        // After `iwt r0`, `reset_latch` leaves `sreg`/`dreg` at 0, so the store
        // sources r0 and the load writes r0.
        let program = [
            0xF7, 0x10, 0x00, // iwt r7,#$0010
            0xF0, 0x34, 0x12, // iwt r0,#$1234
            0x37, // stw (r7)
            0x47, // ldw (r7)
            0x00, // stop
        ];
        let gsu = run(&program, 0x8000);
        assert_eq!(gsu.regs.r[0].data, 0x1234);
        assert_eq!(gsu.read_ram(0x10), 0x34);
        assert_eq!(gsu.read_ram(0x11), 0x12);
    }
}
