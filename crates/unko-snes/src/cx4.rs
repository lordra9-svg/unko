//! Capcom CX4 (Hitachi HG51B S169) coprocessor.
//!
//! Ported from the ares `HG51B` core and the `HitachiDSP` bus wrapper. The DSP
//! is a 24-bit core with a 3 KiB data RAM, a 1 KiB x 24-bit data ROM (the
//! `cx4.rom` firmware) and a two-page 512-byte instruction cache. The program
//! lives in the game ROM; the S-CPU starts it by writing the cache base, PB and
//! PC through the `$6C00`/`$7C00` register window, then polls the registers.
//!
//! Firmware is copyrighted chip data supplied by the host at runtime; it is
//! never embedded here.

use crate::rom::Mapping;

/// One HG51B register file.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cx4Registers {
    /// Program bank (15 bits).
    pub pb: u16,
    /// Program counter (8 bits).
    pub pc: u8,
    /// Negative flag.
    pub n: bool,
    /// Zero flag.
    pub z: bool,
    /// Carry flag.
    pub c: bool,
    /// Overflow flag.
    pub v: bool,
    /// IRQ flag.
    pub i: bool,
    /// Accumulator (24 bits).
    pub a: u32,
    /// Page register (15 bits).
    pub p: u16,
    /// Multiplier result (48 bits).
    pub mul: u64,
    /// Bus memory data register (24 bits).
    pub mdr: u32,
    /// Data-ROM data buffer (24 bits).
    pub rom: u32,
    /// Data-RAM data buffer (24 bits).
    pub ram: u32,
    /// Bus memory address register (24 bits).
    pub mar: u32,
    /// Data-RAM address pointer (24 bits).
    pub dpr: u32,
    /// General-purpose registers (24 bits each).
    pub gpr: [u32; 16],
}

/// Wait-state configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cx4Wait {
    /// ROM wait states (0-7).
    pub rom: u8,
    /// RAM wait states (0-7).
    pub ram: u8,
}

impl Default for Cx4Wait {
    fn default() -> Self {
        Self { rom: 3, ram: 3 }
    }
}

/// Suspend state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cx4Suspend {
    /// Suspend active.
    pub enable: bool,
    /// Remaining duration (0 = indefinite).
    pub duration: u8,
}

/// Instruction-cache state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cx4Cache {
    /// Cache fill enabled.
    pub enable: bool,
    /// Current/other page.
    pub page: bool,
    /// Preload the current page.
    pub preload: bool,
    /// Page locks.
    pub lock: [bool; 2],
    /// Cached base address in bytes.
    pub address: [u32; 2],
    /// Base address in bytes.
    pub base: u32,
    /// Program bank.
    pub pb: u16,
    /// Program counter.
    pub pc: u8,
}

/// DMA state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cx4Dma {
    /// DMA enabled.
    pub enable: bool,
    /// Source address.
    pub source: u32,
    /// Target address.
    pub target: u32,
    /// Length in bytes.
    pub length: u16,
}

/// Deferred bus access state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cx4Bus {
    /// A bus operation is pending.
    pub enable: bool,
    /// The pending operation is a read.
    pub reading: bool,
    /// The pending operation is a write.
    pub writing: bool,
    /// Clocks left before the operation completes.
    pub pending: u8,
    /// Address of the operation.
    pub address: u32,
}

/// HG51B I/O state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cx4Io {
    /// Locked (bus grant to the S-CPU).
    pub lock: bool,
    /// Halted.
    ///
    /// ares `HG51B::power` assigns `io = {}`, which leaves `halt` clear, so the
    /// DSP executes from ROM address 0 until the S-CPU programs it. MX2's
    /// boot-time interface self-test reads these registers before that point
    /// and fails if the DSP has not been running.
    pub halt: bool,
    /// IRQ disable (0 = enabled).
    pub irq: bool,
    /// Single-ROM mode.
    pub rom: bool,
    /// Vector table visible to the S-CPU.
    pub vector: [u8; 32],
    /// Wait states.
    pub wait: Cx4Wait,
    /// Suspend state.
    pub suspend: Cx4Suspend,
    /// Instruction cache.
    pub cache: Cx4Cache,
    /// DMA.
    pub dma: Cx4Dma,
    /// Deferred bus access.
    pub bus: Cx4Bus,
}

impl Default for Cx4Io {
    fn default() -> Self {
        Self {
            lock: false,
            halt: false,
            irq: false,
            rom: true,
            vector: [0; 32],
            wait: Cx4Wait::default(),
            suspend: Cx4Suspend::default(),
            cache: Cx4Cache::default(),
            dma: Cx4Dma::default(),
            bus: Cx4Bus::default(),
        }
    }
}

/// The CX4 / Hitachi HG51B core.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cx4 {
    /// Registers.
    pub regs: Cx4Registers,
    /// I/O and cache/DMA state.
    pub io: Cx4Io,
    /// Two 256-word instruction-cache pages, stored contiguously.
    pub program: Vec<u16>,
    /// Debug counter: instruction-cache reloads performed since power-on.
    pub cache_loads: u64,
    /// 1K x 24-bit data ROM (the `cx4.rom` firmware).
    pub data_rom: Vec<u32>,
    /// 3 KiB data RAM.
    pub data_ram: Vec<u8>,
    /// Eight-entry call stack.
    pub stack: [u16; 8],
    /// Running clock cost of the current `main` call (transient).
    pub cost: u32,
    /// Number of instructions/operations executed (debug).
    pub ticks: u64,
}

/// Errors from installing CX4 firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cx4Error {
    /// The firmware image is not the expected 3 KiB.
    BadFirmwareSize {
        /// Expected size in bytes.
        expected: usize,
        /// Supplied size in bytes.
        found: usize,
    },
}

impl std::fmt::Display for Cx4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFirmwareSize { expected, found } => {
                write!(f, "CX4 firmware is {found} bytes, expected {expected}")
            }
        }
    }
}

impl std::error::Error for Cx4Error {}

impl Default for Cx4 {
    fn default() -> Self {
        Self {
            regs: Cx4Registers::default(),
            io: Cx4Io::default(),
            program: vec![0; 512],
            cache_loads: 0,
            data_rom: Vec::new(),
            data_ram: vec![0; 0xC00],
            stack: [0; 8],
            cost: 0,
            ticks: 0,
        }
    }
}

impl Cx4 {
    /// Parse a 3 KiB `cx4.rom` image (1K x 3-byte little-endian data ROM).
    pub fn from_firmware(firmware: &[u8]) -> Result<Self, Cx4Error> {
        const WORDS: usize = 1024;
        const EXPECTED: usize = WORDS * 3;
        if firmware.len() != EXPECTED {
            return Err(Cx4Error::BadFirmwareSize {
                expected: EXPECTED,
                found: firmware.len(),
            });
        }
        let data_rom = firmware
            .as_chunks::<3>()
            .0
            .iter()
            .map(|b| u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16))
            .collect();
        let mut cx4 = Self {
            data_rom,
            ..Self::default()
        };
        cx4.power();
        Ok(cx4)
    }

    /// Reset the core (ares `HG51B::power`).
    pub fn power(&mut self) {
        self.regs = Cx4Registers::default();
        self.io = Cx4Io::default();
        self.program.fill(0);
        self.cache_loads = 0;
        self.data_ram.fill(0);
        self.stack = [0; 8];
        self.cost = 0;
    }

    /// Whether the DSP currently owns the bus (cache fill, DMA or a pending
    /// bus access).
    pub const fn busy(&self) -> bool {
        self.io.cache.enable || self.io.dma.enable || self.io.bus.pending != 0
    }

    /// Whether the DSP is doing anything at all.
    pub const fn running(&self) -> bool {
        self.io.cache.enable || self.io.dma.enable || self.io.bus.pending != 0 || !self.io.halt
    }

    /// The S-CPU-visible IRQ level.
    pub const fn irq_line(&self) -> bool {
        self.regs.i
    }

    // --- Host registers ---------------------------------------------------

    /// Read one CX4 register byte from an S-CPU offset (`$6C00` window low
    /// 10 bits).
    pub fn read_io(&mut self, address: u16) -> u8 {
        let address = 0x7c00 | (address & 0x03ff);
        match address {
            0x7f40 => self.io.dma.source as u8,
            0x7f41 => (self.io.dma.source >> 8) as u8,
            0x7f42 => (self.io.dma.source >> 16) as u8,
            0x7f43 => self.io.dma.length as u8,
            0x7f44 => (self.io.dma.length >> 8) as u8,
            0x7f45 => self.io.dma.target as u8,
            0x7f46 => (self.io.dma.target >> 8) as u8,
            0x7f47 => (self.io.dma.target >> 16) as u8,
            0x7f48 => u8::from(self.io.cache.page),
            0x7f49 => self.io.cache.base as u8,
            0x7f4a => (self.io.cache.base >> 8) as u8,
            0x7f4b => (self.io.cache.base >> 16) as u8,
            0x7f4c => u8::from(self.io.cache.lock[0]) | (u8::from(self.io.cache.lock[1]) << 1),
            0x7f4d => self.io.cache.pb as u8,
            0x7f4e => (self.io.cache.pb >> 8) as u8,
            0x7f4f => self.io.cache.pc,
            0x7f50 => (self.io.wait.ram & 7) | ((self.io.wait.rom & 7) << 4),
            0x7f51 => u8::from(self.io.irq),
            0x7f52 => u8::from(self.io.rom),
            0x7f53..=0x7f57 | 0x7f59 | 0x7f5b..=0x7f5f => {
                u8::from(self.io.suspend.enable)
                    | (u8::from(self.regs.i) << 1)
                    | (u8::from(self.running()) << 6)
                    | (u8::from(self.busy()) << 7)
            }
            0x7f60..=0x7f7f => self.io.vector[usize::from(address & 0x1f)],
            0x7f80..=0x7faf | 0x7fc0..=0x7fef => {
                let address = (address & 0x3f) as usize;
                let value = self.regs.gpr[address / 3];
                (value >> ((address % 3) * 8)) as u8
            }
            _ => 0,
        }
    }

    /// Write one CX4 register byte from an S-CPU offset.
    pub fn write_io(&mut self, address: u16, data: u8) {
        let address = 0x7c00 | (address & 0x03ff);
        self.trace("I", address, data);
        match address {
            0x7f40 => self.io.dma.source = (self.io.dma.source & 0xffff00) | u32::from(data),
            0x7f41 => self.io.dma.source = (self.io.dma.source & 0xff00ff) | (u32::from(data) << 8),
            0x7f42 => {
                self.io.dma.source = (self.io.dma.source & 0x00ffff) | (u32::from(data) << 16)
            }
            0x7f43 => self.io.dma.length = (self.io.dma.length & 0xff00) | u16::from(data),
            0x7f44 => self.io.dma.length = (self.io.dma.length & 0x00ff) | (u16::from(data) << 8),
            0x7f45 => self.io.dma.target = (self.io.dma.target & 0xffff00) | u32::from(data),
            0x7f46 => self.io.dma.target = (self.io.dma.target & 0xff00ff) | (u32::from(data) << 8),
            0x7f47 => {
                self.io.dma.target = (self.io.dma.target & 0x00ffff) | (u32::from(data) << 16);
                if self.io.halt {
                    self.io.dma.enable = true;
                }
            }
            0x7f48 => {
                self.io.cache.page = data & 1 != 0;
                if insn_logging() {
                    let halt = u8::from(self.io.halt);
                    self.log_event(
                        "W48",
                        u32::from(self.io.cache.pb),
                        u32::from(data) | (u32::from(halt) << 8),
                    );
                    let lead = DEBUG_CPU_LEAD.load(std::sync::atomic::Ordering::Relaxed);
                    self.log_event("L48", lead as u32 & 0xffff, 0);
                }
                if insn_logging() {
                    let lead = DEBUG_CPU_LEAD.load(std::sync::atomic::Ordering::Relaxed);
                    self.log_event("LEAD", lead as u32 & 0xffff, 0x48);
                }
                if self.io.halt {
                    self.regs.pb = self.io.cache.pb;
                    self.io.cache.preload = true;
                    self.io.cache.enable = true;
                }
            }
            0x7f49 => self.io.cache.base = (self.io.cache.base & 0xffff00) | u32::from(data),
            0x7f4a => self.io.cache.base = (self.io.cache.base & 0xff00ff) | (u32::from(data) << 8),
            0x7f4b => {
                self.io.cache.base = (self.io.cache.base & 0x00ffff) | (u32::from(data) << 16)
            }
            0x7f4c => {
                self.io.cache.lock[0] = data & 1 != 0;
                self.io.cache.lock[1] = data & 2 != 0;
            }
            0x7f4d => self.io.cache.pb = (self.io.cache.pb & 0xff00) | u16::from(data),
            0x7f4e => self.io.cache.pb = (self.io.cache.pb & 0x00ff) | (u16::from(data) << 8),
            0x7f4f => {
                // Debug: the state at each job-request write, to compare the
                // coprocessor's phase with the reference core's.
                if let Some(path) = std::env::var_os("CX4_JOB_LOG") {
                    use std::io::Write;
                    let frame = DEBUG_FRAME.load(std::sync::atomic::Ordering::Relaxed);
                    let from: u32 = std::env::var("CX4_JOB_FROM")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let to: u32 = std::env::var("CX4_JOB_TO")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(u32::MAX);
                    if (from..=to).contains(&frame) {
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                        {
                            let _ = writeln!(
                                f,
                                "{} halt={} pb={:04X} pc={:02X} run={} busy={} ticks={} cpu_pc={:06X} sp={:02X} stack={:012X}",
                                frame,
                                u8::from(self.io.halt),
                                self.io.cache.pb,
                                self.io.cache.pc,
                                u8::from(self.running()),
                                u8::from(self.busy()),
                                self.ticks,
                                DEBUG_PC.load(std::sync::atomic::Ordering::Relaxed),
                                DEBUG_SP.load(std::sync::atomic::Ordering::Relaxed) & 0xFF,
                                DEBUG_STACK.load(std::sync::atomic::Ordering::Relaxed)
                            );
                        }
                    }
                }
                self.io.cache.pc = data;
                if insn_logging() {
                    let halt = u8::from(self.io.halt);
                    let pb = self.io.cache.pb;
                    let ticks = self.ticks as u32;
                    self.log_event("T4F", u32::from(pb) | (u32::from(halt) << 24), ticks);
                    let lead = DEBUG_CPU_LEAD.load(std::sync::atomic::Ordering::Relaxed);
                    self.log_event("LEAD", lead as u32 & 0xffff, u32::from(pb));
                }
                if self.io.halt {
                    DEBUG_JOB_STARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // Debug: a job starts here, so the data RAM at this point is
                    // content-addressed (no frame-boundary phase involved). The
                    // Nth job start must hash identically in both cores.
                    if let Some(path) = std::env::var_os("CX4_DRAM_HASH") {
                        use std::io::Write;
                        static JOB: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(0);
                        let job = JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
                        for b in &self.data_ram {
                            hash ^= u64::from(*b);
                            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                        }
                        let mut greg: u64 = 0xcbf2_9ce4_8422_2325;
                        for g in &self.regs.gpr {
                            for b in g.to_le_bytes().iter().take(3) {
                                greg ^= u64::from(*b);
                                greg = greg.wrapping_mul(0x0000_0100_0000_01b3);
                            }
                        }
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                        {
                            let _ = writeln!(
                                f,
                                "J {job} {hash:016x} g={greg:016x} w={:016x} t={} pb={:04X} pc={:02X}",
                                DEBUG_WRAM_HASH.load(std::sync::atomic::Ordering::Relaxed),
                                DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed),
                                self.io.cache.pb,
                                self.io.cache.pc
                            );
                        }
                    }
                    self.io.halt = false;
                    self.regs.pb = self.io.cache.pb;
                    self.regs.pc = self.io.cache.pc;
                }
            }
            0x7f50 => {
                self.io.wait.ram = data & 7;
                self.io.wait.rom = (data >> 4) & 7;
            }
            0x7f51 => {
                self.io.irq = data & 1 != 0;
                if self.io.irq {
                    self.regs.i = false;
                }
            }
            0x7f52 => self.io.rom = data & 1 != 0,
            0x7f53 => {
                self.io.lock = false;
                self.io.halt = true;
            }
            0x7f55 => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 0;
            }
            0x7f56 => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 32;
            }
            0x7f57 => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 64;
            }
            0x7f58 => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 96;
            }
            0x7f59 => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 128;
            }
            0x7f5a => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 160;
            }
            0x7f5b => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 192;
            }
            0x7f5c => {
                self.io.suspend.enable = true;
                self.io.suspend.duration = 224;
            }
            0x7f5d => self.io.suspend.enable = false,
            0x7f5e => self.regs.i = false,
            0x7f60..=0x7f7f => self.io.vector[usize::from(address & 0x1f)] = data,
            0x7f80..=0x7faf | 0x7fc0..=0x7fef => {
                let address = (address & 0x3f) as usize;
                let shift = (address % 3) * 8;
                let slot = &mut self.regs.gpr[address / 3];
                *slot = (*slot & !(0xff << shift)) | (u32::from(data) << shift);
            }
            _ => {}
        }
    }

    /// Read the CX4 data RAM through the S-CPU `$6000` window.
    pub fn read_dram(&self, address: u32) -> u8 {
        let address = (address & 0xfff) as usize;
        if address >= 0xc00 {
            return 0;
        }
        self.data_ram[address]
    }

    /// Write the CX4 data RAM through the S-CPU `$6000` window.
    pub fn write_dram(&mut self, address: u32, data: u8) {
        let address = (address & 0xfff) as usize;
        if address < 0xc00 {
            self.trace("D", address as u16, data);
            if let Some(path) = std::env::var_os("CX4_WR_LOG")
                .filter(|_| matches!(address, 0x71e | 0x71f | 0x7d0 | 0x7d1))
            {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "CPU {:04X} = {:02X}", address, data);
                }
            }
            self.data_ram[address] = data;
            // Debug: hash the data RAM after each S-CPU write into the
            // `$6000`/`$7000` window (`CX4_CPUWR_HASH`), so the CPU's uploads
            // can be compared with the reference core's.
            if let Some(path) = std::env::var_os("CX4_CPUWR_HASH") {
                use std::io::Write;
                let frame = DEBUG_FRAME.load(std::sync::atomic::Ordering::Relaxed);
                let from: u32 = std::env::var("CX4_CPUWR_FROM")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let to: u32 = std::env::var("CX4_CPUWR_TO")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(u32::MAX);
                if (from..=to).contains(&frame) {
                    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
                    for b in &self.data_ram {
                        hash ^= u64::from(*b);
                        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                    }
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        let _ = writeln!(
                            f,
                            "{} {} {:03X}={:02X} {:016x} pb={:04X} pc={:02X} run={} busy={} ticks={} halts={}",
                            DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed),
                            frame,
                            address,
                            data,
                            hash,
                            self.regs.pb,
                            self.regs.pc,
                            u8::from(self.running()),
                            u8::from(self.busy()),
                            self.ticks,
                            DEBUG_HALTS.load(std::sync::atomic::Ordering::Relaxed)
                        );
                    }
                }
            }
        }
    }

    /// Classify an S-CPU address as a CX4 register-window access.
    ///
    /// Returns the low 10-bit window offset for `$6C00-$6FFF`/`$7C00-$7FFF`
    /// in banks `$00-$3F`/`$80-$BF`.
    pub fn io_window(address: u32) -> Option<u16> {
        let bank = (address >> 16) & 0x7f;
        let offset = address & 0xffff;
        if bank <= 0x3f && (0x6c00..=0x6fff).contains(&offset) {
            return Some((offset & 0x3ff) as u16);
        }
        if bank <= 0x3f && (0x7c00..=0x7fff).contains(&offset) {
            return Some((offset & 0x3ff) as u16);
        }
        None
    }

    /// Classify an S-CPU address as a CX4 data-RAM access.
    pub fn dram_window(address: u32) -> Option<u32> {
        let bank = (address >> 16) & 0x7f;
        let offset = address & 0xffff;
        if bank <= 0x3f && (0x6000..=0x6bff).contains(&offset) {
            return Some(offset & 0xfff);
        }
        if bank <= 0x3f && (0x7000..=0x7bff).contains(&offset) {
            return Some(offset & 0xfff);
        }
        None
    }

    // --- Core -------------------------------------------------------------

    /// Run one DSP scheduling step and return the HG51B clocks it consumed.
    pub fn run_one(&mut self, rom: &[u8], sram: &[u8]) -> u32 {
        self.cost = 0;
        if self.io.lock {
            self.step(rom, sram, 1);
        } else if self.io.suspend.enable {
            self.suspend(rom, sram);
        } else if self.io.cache.enable {
            self.cache(rom, sram);
        } else if self.io.dma.enable {
            self.dma(rom, sram);
        } else if self.io.halt {
            self.step(rom, sram, 1);
        } else {
            self.execute(rom, sram);
        }
        if spin_logging() {
            DEBUG_CYCLES.fetch_add(u64::from(self.cost), std::sync::atomic::Ordering::Relaxed);
        }
        self.cost
    }

    fn step(&mut self, rom: &[u8], sram: &[u8], clocks: u32) {
        self.cost += clocks;
        if self.io.bus.enable {
            if u32::from(self.io.bus.pending) > clocks {
                self.io.bus.pending -= clocks as u8;
            } else {
                self.io.bus.enable = false;
                self.io.bus.pending = 0;
                if self.io.bus.reading {
                    self.io.bus.reading = false;
                    self.regs.mdr = u32::from(self.read_bus(rom, sram, self.io.bus.address));
                }
                if self.io.bus.writing {
                    self.io.bus.writing = false;
                    let data = self.regs.mdr as u8;
                    self.write_bus(rom, sram, self.io.bus.address, data);
                }
            }
        }
    }

    fn suspend(&mut self, rom: &[u8], sram: &[u8]) {
        if self.io.suspend.duration == 0 {
            self.step(rom, sram, 1);
            return;
        }
        let duration = u32::from(self.io.suspend.duration);
        self.step(rom, sram, duration);
        self.io.suspend.duration = 0;
        self.io.suspend.enable = false;
    }

    fn wait(&self, address: u32) -> u32 {
        if self.address_rom(address).is_some() {
            return 1 + u32::from(self.io.wait.rom);
        }
        if self.address_ram(address).is_some() {
            return 1 + u32::from(self.io.wait.ram);
        }
        0
    }

    fn cache(&mut self, rom: &[u8], sram: &[u8]) -> bool {
        let mut address = self
            .io
            .cache
            .base
            .wrapping_add(u32::from(self.regs.pb) << 9);
        let page = usize::from(self.io.cache.page);

        if !self.io.cache.preload {
            if self.io.cache.address[page] == address {
                self.io.cache.enable = false;
                return true;
            }
            self.io.cache.page = !self.io.cache.page;
            let page = usize::from(self.io.cache.page);
            if self.io.cache.address[page] == address {
                self.io.cache.enable = false;
                return true;
            }
            if self.io.cache.lock[page] {
                self.io.cache.page = !self.io.cache.page;
            }
            let page = usize::from(self.io.cache.page);
            if self.io.cache.lock[page] {
                self.io.cache.enable = false;
                return false;
            }
        }
        self.io.cache.preload = false;
        let page = usize::from(self.io.cache.page);
        self.io.cache.address[page] = address;
        if insn_logging() {
            self.log_prep(address);
        }
        let debug_load = insn_logging();

        for offset in 0..256 {
            let wait = self.wait(address);
            self.step(rom, sram, wait);
            let low = self.read_bus(rom, sram, address);
            address = address.wrapping_add(1);
            let wait = self.wait(address);
            self.step(rom, sram, wait);
            let high = self.read_bus(rom, sram, address);
            address = address.wrapping_add(1);
            self.program[page * 256 + offset] = u16::from_le_bytes([low, high]);
        }
        self.io.cache.enable = false;
        self.cache_loads = self.cache_loads.wrapping_add(1);
        if debug_load {
            self.log_event("CACHE", u32::from(self.regs.pb), address);
        }
        true
    }

    fn dma(&mut self, rom: &[u8], sram: &[u8]) {
        for offset in 0..u32::from(self.io.dma.length) {
            let source = self.io.dma.source.wrapping_add(offset);
            let target = self.io.dma.target.wrapping_add(offset);
            if self.address_rom(source).is_some() && self.address_rom(target).is_some() {
                self.io.lock = true;
                return;
            }
            if self.address_ram(source).is_some() && self.address_ram(target).is_some() {
                self.io.lock = true;
                return;
            }
            let wait = self.wait(source);
            self.step(rom, sram, wait);
            let data = self.read_bus(rom, sram, source);
            let wait = self.wait(target);
            self.step(rom, sram, wait);
            self.write_bus(rom, sram, target, data);
        }
        self.io.dma.enable = false;
    }

    fn execute(&mut self, rom: &[u8], sram: &[u8]) {
        if !self.cache(rom, sram) {
            self.halt();
            return;
        }
        let page = usize::from(self.io.cache.page);
        let pc = usize::from(self.regs.pc);
        let opcode = self.program[page * 256 + pc];
        self.log_instruction(opcode);
        self.advance(rom, sram);
        self.step(rom, sram, 1);
        self.ticks = self.ticks.wrapping_add(1);
        self.decode(opcode, rom, sram);
    }

    fn advance(&mut self, rom: &[u8], sram: &[u8]) {
        self.regs.pc = self.regs.pc.wrapping_add(1);
        if self.regs.pc == 0 {
            if self.io.cache.page {
                self.halt();
                return;
            }
            self.io.cache.page = true;
            if self.io.cache.lock[1] {
                self.halt();
                return;
            }
            self.regs.pb = self.regs.p;
            if !self.cache(rom, sram) {
                self.halt();
            }
        }
    }

    fn halt(&mut self) {
        DEBUG_HALTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.io.halt = true;
        if !self.io.irq {
            self.regs.i = true;
        }
        // Debug: hash the data RAM at the halt instruction, i.e. the job's
        // output before the S-CPU reads it.
        if let Some(path) = std::env::var_os("CX4_HALT_HASH") {
            use std::io::Write;
            static JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let job = JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for b in &self.data_ram {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "H {job} {hash:016x}");
            }
        }
    }

    // --- Bus mapping ------------------------------------------------------

    fn address_rom(&self, address: u32) -> Option<u32> {
        // Mapping 0 (all CX4 boards): LoROM upper halves and $c0-$ff linear.
        if (address & 0x40_8000) == 0x00_8000 || (address & 0xc0_0000) == 0xc0_0000 {
            let linear = ((address & 0x3f_0000) >> 1) | (address & 0x7fff);
            return Some(linear & 0x1f_ffff);
        }
        None
    }

    fn address_ram(&self, address: u32) -> Option<u32> {
        if (address & 0xf8_8000) == 0x70_0000 {
            let linear = ((address & 0x07_0000) >> 1) | (address & 0x7fff);
            return Some(linear & 0x03_ffff);
        }
        None
    }

    fn address_dram(&self, address: u32) -> Option<u32> {
        if (address & 0x40_e000) == 0x00_6000 && (address & 0x0c00) != 0x0c00 {
            return Some(address & 0x0fff);
        }
        None
    }

    fn address_io(&self, address: u32) -> Option<u32> {
        if (address & 0x40_ec00) == 0x00_6c00 {
            return Some(address & 0x03ff);
        }
        None
    }

    fn read_bus(&mut self, rom: &[u8], sram: &[u8], address: u32) -> u8 {
        if let Some(linear) = self.address_rom(address) {
            if rom.is_empty() {
                return 0;
            }
            return rom[(linear as usize) % rom.len()];
        }
        if let Some(linear) = self.address_ram(address) {
            if sram.is_empty() {
                return 0;
            }
            return sram[(linear as usize) % sram.len()];
        }
        if let Some(linear) = self.address_dram(address) {
            let index = (linear & 0xfff) as usize;
            return if index < 0xc00 {
                self.data_ram[index]
            } else {
                0
            };
        }
        if let Some(io) = self.address_io(address) {
            return self.read_io(io as u16);
        }
        0
    }

    fn write_bus(&mut self, _rom: &[u8], _sram: &[u8], address: u32, data: u8) {
        if let Some(linear) = self.address_dram(address) {
            let index = (linear & 0xfff) as usize;
            if index < 0xc00 {
                self.data_ram[index] = data;
            }
            return;
        }
        if let Some(io) = self.address_io(address) {
            self.write_io(io as u16, data);
        }
        // ROM and SRAM writes are ignored.
    }

    // --- Registers --------------------------------------------------------

    fn read_register(&mut self, address: u8) -> u32 {
        match address {
            0x01 => (self.regs.mul >> 24) as u32 & 0xff_ffff,
            0x02 => self.regs.mul as u32 & 0xff_ffff,
            0x03 => self.regs.mdr,
            0x08 => self.regs.rom,
            0x0c => self.regs.ram,
            0x13 => self.regs.mar,
            0x1c => self.regs.dpr,
            0x20 => u32::from(self.regs.pc),
            0x28 => u32::from(self.regs.p),
            0x2e => {
                self.io.bus.enable = true;
                self.io.bus.reading = true;
                self.io.bus.pending = self.io.wait.rom;
                self.io.bus.address = self.regs.mar;
                0
            }
            0x2f => {
                self.io.bus.enable = true;
                self.io.bus.reading = true;
                self.io.bus.pending = self.io.wait.ram;
                self.io.bus.address = self.regs.mar;
                0
            }
            0x50 => 0x00_0000,
            0x51 => 0xff_ffff,
            0x52 => 0x00_ff00,
            0x53 => 0xff_0000,
            0x54 => 0x00_ffff,
            0x55 => 0xff_ff00,
            0x56 => 0x80_0000,
            0x57 => 0x7f_ffff,
            0x58 => 0x00_8000,
            0x59 => 0x00_7fff,
            0x5a => 0xff_7fff,
            0x5b => 0xff_ff7f,
            0x5c => 0x01_0000,
            0x5d => 0xfe_ffff,
            0x5e => 0x00_0100,
            0x5f => 0x00_feff,
            0x60..=0x6f => self.regs.gpr[usize::from(address & 0x0f)],
            0x70..=0x7f => self.regs.gpr[usize::from(address & 0x0f)],
            _ => 0,
        }
    }

    fn write_register(&mut self, address: u8, data: u32) {
        let data = data & 0xff_ffff;
        match address {
            0x01 => self.regs.mul = (self.regs.mul & 0x00_0000_00ff_ffff) | (u64::from(data) << 24),
            0x02 => self.regs.mul = (self.regs.mul & 0x00_ffff_ff00_0000) | u64::from(data),
            0x03 => self.regs.mdr = data,
            0x08 => self.regs.rom = data,
            0x0c => self.regs.ram = data,
            0x13 => self.regs.mar = data,
            0x1c => self.regs.dpr = data,
            0x20 => self.regs.pc = data as u8,
            0x28 => self.regs.p = data as u16 & 0x7fff,
            0x2e => {
                self.io.bus.enable = true;
                self.io.bus.writing = true;
                self.io.bus.pending = self.io.wait.rom;
                self.io.bus.address = self.regs.mar;
            }
            0x2f => {
                self.io.bus.enable = true;
                self.io.bus.writing = true;
                self.io.bus.pending = self.io.wait.ram;
                self.io.bus.address = self.regs.mar;
            }
            0x60..=0x6f => self.regs.gpr[usize::from(address & 0x0f)] = data,
            0x70..=0x7f => self.regs.gpr[usize::from(address & 0x0f)] = data,
            _ => {}
        }
    }

    // --- ALU --------------------------------------------------------------

    fn set_nz(&mut self, value: u32) {
        self.regs.n = value & 0x80_0000 != 0;
        self.regs.z = value & 0xff_ffff == 0;
    }

    fn algorithm_add(&mut self, x: u32, y: u32) -> u32 {
        let z = x.wrapping_add(y);
        let result = z & 0xff_ffff;
        self.set_nz(result);
        self.regs.c = z > 0xff_ffff;
        self.regs.v = (!(x ^ y) & (x ^ z) & 0x80_0000) != 0;
        result
    }

    fn algorithm_sub(&mut self, x: u32, y: u32) -> u32 {
        let z = x.wrapping_sub(y);
        let result = z & 0xff_ffff;
        self.set_nz(result);
        // ares: `r.c = z >= 0` on the signed 32-bit difference.
        self.regs.c = (z as i32) >= 0;
        self.regs.v = (!(x ^ y) & (x ^ z) & 0x80_0000) != 0;
        result
    }

    fn algorithm_and(&mut self, x: u32, y: u32) -> u32 {
        let value = (x & y) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_or(&mut self, x: u32, y: u32) -> u32 {
        let value = (x | y) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_xor(&mut self, x: u32, y: u32) -> u32 {
        let value = (x ^ y) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_xnor(&mut self, x: u32, y: u32) -> u32 {
        let value = ((!x) ^ y) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_asr(&mut self, a: u32, s: u32) -> u32 {
        let s = if s > 24 { 0 } else { s };
        // ares shifts `(i24)a`, i.e. with bit 23 as the sign bit.
        let value = (Self::sign_extend_24(a) >> s) as u32 & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_shl(&mut self, a: u32, s: u32) -> u32 {
        let s = if s > 24 { 0 } else { s };
        let value = (a << s) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_shr(&mut self, a: u32, s: u32) -> u32 {
        let s = if s > 24 { 0 } else { s };
        let value = (a >> s) & 0xff_ffff;
        self.set_nz(value);
        value
    }

    fn algorithm_ror(&mut self, a: u32, s: u32) -> u32 {
        let s = if s > 24 { 0 } else { s };
        let value = if s == 0 {
            a & 0xff_ffff
        } else {
            ((a >> s) | (a << (24 - s))) & 0xff_ffff
        };
        self.set_nz(value);
        value
    }

    /// Sign-extend a 24-bit value the way ares' `i24` type does.
    const fn sign_extend_24(value: u32) -> i64 {
        let low = (value & 0xff_ffff) as i32;
        (low << 8 >> 8) as i64
    }

    fn algorithm_sx(&mut self, value: u32) -> u32 {
        self.set_nz(value & 0xff_ffff);
        value & 0xff_ffff
    }

    // --- Stack ------------------------------------------------------------

    fn push(&mut self) {
        self.stack.copy_within(0..7, 1);
        self.stack[0] = (self.regs.pb << 8) | u16::from(self.regs.pc);
    }

    fn pull(&mut self) {
        let value = self.stack[0];
        self.stack.copy_within(1..8, 0);
        self.stack[7] = 0;
        self.regs.pb = (value >> 8) & 0x7fff;
        self.regs.pc = value as u8;
    }

    // --- Decode -----------------------------------------------------------

    fn decode(&mut self, opcode: u16, rom: &[u8], sram: &[u8]) {
        let data = opcode & 0xff;
        let far = (opcode >> 9) & 1 != 0;
        let shift = match (opcode >> 8) & 0x3 {
            0 => 0,
            1 => 1,
            2 => 8,
            _ => 16,
        };
        let reg = (opcode & 0x7f) as u8;
        match opcode >> 12 {
            0x0 => match (opcode >> 10) & 0x3 {
                0 | 1 => {}
                2 => self.jmp(rom, sram, data as u8, far, true),
                _ => {
                    let take = self.regs.z;
                    self.jmp(rom, sram, data as u8, far, take);
                }
            },
            0x1 => match (opcode >> 10) & 0x3 {
                0 => {
                    let take = self.regs.c;
                    self.jmp(rom, sram, data as u8, far, take);
                }
                1 => {
                    let take = self.regs.n;
                    self.jmp(rom, sram, data as u8, far, take);
                }
                2 => {
                    let take = self.regs.v;
                    self.jmp(rom, sram, data as u8, far, take);
                }
                _ => self.wait_bus(rom, sram),
            },
            0x2 => match (opcode >> 10) & 0x3 {
                0 => {}
                1 => {
                    let flag = match (opcode >> 8) & 0x3 {
                        0 => self.regs.v,
                        1 => self.regs.c,
                        2 => self.regs.z,
                        _ => self.regs.n,
                    };
                    let take = opcode & 1 != 0;
                    if flag == take {
                        self.regs.pc = self.regs.pc.wrapping_add(1);
                        self.step(rom, sram, 1);
                    }
                }
                2 => self.jsr(rom, sram, data as u8, far, true),
                _ => {
                    let take = self.regs.z;
                    self.jsr(rom, sram, data as u8, far, take);
                }
            },
            0x3 => match (opcode >> 10) & 0x3 {
                0 => {
                    let take = self.regs.c;
                    self.jsr(rom, sram, data as u8, far, take);
                }
                1 => {
                    let take = self.regs.n;
                    self.jsr(rom, sram, data as u8, far, take);
                }
                2 => {
                    let take = self.regs.v;
                    self.jsr(rom, sram, data as u8, far, take);
                }
                _ => {
                    self.pull();
                    self.step(rom, sram, 2);
                }
            },
            0x4 => match (opcode >> 10) & 0x3 {
                0 => self.regs.mar = self.regs.mar.wrapping_add(1) & 0xff_ffff,
                1 => {}
                2 => {
                    let value = self.read_register(reg);
                    self.algorithm_sub(value, self.regs.a << shift);
                }
                _ => {
                    self.algorithm_sub(u32::from(data), self.regs.a << shift);
                }
            },
            0x5 => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.algorithm_sub(self.regs.a << shift, value);
                }
                1 => {
                    self.algorithm_sub(self.regs.a << shift, u32::from(data));
                }
                2 => match (opcode >> 8) & 0x3 {
                    0 => {}
                    1 => {
                        let value = (self.regs.a as i8) as i32 as u32 & 0xff_ffff;
                        self.regs.a = self.algorithm_sx(value);
                    }
                    2 => {
                        let value = (self.regs.a as i16) as i32 as u32 & 0xff_ffff;
                        self.regs.a = self.algorithm_sx(value);
                    }
                    _ => {}
                },
                _ => {}
            },
            0x6 => match (opcode >> 8) & 0xf {
                0 => self.regs.a = self.read_register(reg),
                1 => self.regs.mdr = self.read_register(reg),
                2 => self.regs.mar = self.read_register(reg),
                3 => self.regs.p = self.regs.gpr[usize::from(opcode & 0xf)] as u16 & 0x7fff,
                4 => self.regs.a = u32::from(data),
                5 => self.regs.mdr = u32::from(data),
                6 => self.regs.mar = u32::from(data),
                7 => self.regs.p = data & 0x7fff,
                8 => self.rdram_a(0),
                9 => self.rdram_a(1),
                10 => self.rdram_a(2),
                12 => self.rdram_imm(0, data as u8),
                13 => self.rdram_imm(1, data as u8),
                14 => self.rdram_imm(2, data as u8),
                _ => {}
            },
            0x7 => match (opcode >> 8) & 0xf {
                0..=3 => self.regs.rom = self.data_rom[usize::from(self.regs.a as u16 & 0x3ff)],
                4..=7 => self.regs.rom = self.data_rom[usize::from(opcode & 0x3ff)],
                12 => {
                    self.regs.p = (self.regs.p & 0x7f00) | u16::from(data as u8);
                }
                13 => {
                    self.regs.p = (self.regs.p & 0x00ff) | (u16::from((opcode & 0x7f) as u8) << 8);
                }
                _ => {}
            },
            0x8 => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_add(self.regs.a << shift, value);
                }
                1 => {
                    self.regs.a = self.algorithm_add(self.regs.a << shift, u32::from(data));
                }
                2 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_sub(value, self.regs.a << shift);
                }
                _ => {
                    self.regs.a = self.algorithm_sub(u32::from(data), self.regs.a << shift);
                }
            },
            0x9 => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_sub(self.regs.a << shift, value);
                }
                1 => {
                    self.regs.a = self.algorithm_sub(self.regs.a << shift, u32::from(data));
                }
                2 => {
                    let value = self.read_register(reg);
                    let product = Self::sign_extend_24(self.regs.a) * Self::sign_extend_24(value);
                    self.regs.mul = product as u64 & 0xffff_ffff_ffff;
                }
                _ => {
                    // ares `instructionMUL(n8 imm)`: the immediate is unsigned.
                    let product = Self::sign_extend_24(self.regs.a) * i64::from(data as u8);
                    self.regs.mul = product as u64 & 0xffff_ffff_ffff;
                }
            },
            0xa => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_xnor(self.regs.a << shift, value);
                }
                1 => {
                    self.regs.a = self.algorithm_xnor(self.regs.a << shift, u32::from(data));
                }
                2 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_xor(self.regs.a << shift, value);
                }
                _ => {
                    self.regs.a = self.algorithm_xor(self.regs.a << shift, u32::from(data));
                }
            },
            0xb => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_and(self.regs.a << shift, value);
                }
                1 => {
                    self.regs.a = self.algorithm_and(self.regs.a << shift, u32::from(data));
                }
                2 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_or(self.regs.a << shift, value);
                }
                _ => {
                    self.regs.a = self.algorithm_or(self.regs.a << shift, u32::from(data));
                }
            },
            0xc => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_shr(self.regs.a, value);
                }
                1 => {
                    self.regs.a = self.algorithm_shr(self.regs.a, u32::from(opcode & 0x1f));
                }
                2 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_asr(self.regs.a, value);
                }
                _ => {
                    self.regs.a = self.algorithm_asr(self.regs.a, u32::from(opcode & 0x1f));
                }
            },
            0xd => match (opcode >> 10) & 0x3 {
                0 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_ror(self.regs.a, value);
                }
                1 => {
                    self.regs.a = self.algorithm_ror(self.regs.a, u32::from(opcode & 0x1f));
                }
                2 => {
                    let value = self.read_register(reg);
                    self.regs.a = self.algorithm_shl(self.regs.a, value);
                }
                _ => {
                    self.regs.a = self.algorithm_shl(self.regs.a, u32::from(opcode & 0x1f));
                }
            },
            0xe => match (opcode >> 8) & 0xf {
                0 => {
                    let value = self.regs.a;
                    self.write_register(reg, value);
                }
                1 => {
                    let value = self.regs.mdr;
                    self.write_register(reg, value);
                }
                8 => self.wrram_a(0),
                9 => self.wrram_a(1),
                10 => self.wrram_a(2),
                12 => self.wrram_imm(0, data as u8),
                13 => self.wrram_imm(1, data as u8),
                14 => self.wrram_imm(2, data as u8),
                _ => {}
            },
            0xf => match (opcode >> 10) & 0x3 {
                0 => {
                    let index = usize::from(opcode & 0xf);
                    std::mem::swap(&mut self.regs.a, &mut self.regs.gpr[index]);
                }
                1 => {}
                2 => {
                    self.regs.a = 0;
                    self.regs.p = 0;
                    self.regs.ram = 0;
                    self.regs.dpr = 0;
                }
                _ => self.halt(),
            },
            _ => {}
        }
    }

    fn wait_bus(&mut self, rom: &[u8], sram: &[u8]) {
        if self.io.bus.enable {
            let pending = u32::from(self.io.bus.pending);
            self.step(rom, sram, pending);
        }
    }

    fn jmp(&mut self, rom: &[u8], sram: &[u8], data: u8, far: bool, take: bool) {
        if !take {
            return;
        }
        if far {
            self.regs.pb = self.regs.p;
        }
        self.regs.pc = data;
        self.step(rom, sram, 2);
    }

    fn jsr(&mut self, rom: &[u8], sram: &[u8], data: u8, far: bool, take: bool) {
        if !take {
            return;
        }
        self.push();
        if far {
            self.regs.pb = self.regs.p;
        }
        self.regs.pc = data;
        self.step(rom, sram, 2);
    }

    fn rdram_a(&mut self, byte: u32) {
        let mut address = self.regs.a as u16 & 0x0fff;
        if address >= 0xc00 {
            address -= 0x400;
        }
        let value = u32::from(self.data_ram[usize::from(address)]);
        let shift = byte * 8;
        self.regs.ram = (self.regs.ram & !(0xff << shift)) | (value << shift);
    }

    fn rdram_imm(&mut self, byte: u32, imm: u8) {
        let mut address = self.regs.dpr.wrapping_add(u32::from(imm)) as u16 & 0x0fff;
        if address >= 0xc00 {
            address -= 0x400;
        }
        let value = u32::from(self.data_ram[usize::from(address)]);
        let shift = byte * 8;
        self.regs.ram = (self.regs.ram & !(0xff << shift)) | (value << shift);
    }

    fn wrram_a(&mut self, byte: u32) {
        let mut address = self.regs.a as u16 & 0x0fff;
        if address >= 0xc00 {
            address -= 0x400;
        }
        let value = (self.regs.ram >> (byte * 8)) as u8;
        self.log_wrram(address, byte);
        self.data_ram[usize::from(address)] = value;
    }

    /// Debug: log one executed instruction (`CX4_INSN_LOG=/path`), for
    /// instruction-by-instruction comparison against ares. `snes-info` keeps
    /// [`DEBUG_FRAME`] current and reads the start frame from
    /// `CX4_INSN_FROM`.
    /// Debug: log a cache reload request with the page-cache state *before* the
    /// page is overwritten, so a miss can be compared against a reference.
    fn log_prep(&self, request: u32) {
        let Some(path) = std::env::var_os("CX4_INSN_LOG") else {
            return;
        };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let clock = DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed);
            let locks = u8::from(self.io.cache.lock[0]) | (u8::from(self.io.cache.lock[1]) << 1);
            let _ = writeln!(
                f,
                "t={clock} prep pb={:04X} req={:06X} page={} a0={:06X} a1={:06X} l={} w={}",
                self.regs.pb,
                request,
                u8::from(self.io.cache.page),
                self.io.cache.address[0],
                self.io.cache.address[1],
                locks,
                self.io.wait.rom
            );
        }
    }

    /// Debug: append a non-instruction event to the instruction log.
    fn log_event(&self, kind: &str, value: u32, extra: u32) {
        let Some(path) = std::env::var_os("CX4_INSN_LOG") else {
            return;
        };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let clock = DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed);
            let _ = writeln!(f, "t={clock} {kind} v={value:06X} x={extra:06X}");
        }
    }

    fn log_instruction(&self, opcode: u16) {
        if !insn_logging() {
            return;
        }
        let frame = DEBUG_FRAME.load(std::sync::atomic::Ordering::Relaxed);
        if frame < DEBUG_INSN_FROM.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(path) = std::env::var_os("CX4_INSN_LOG") else {
            return;
        };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            // FNV-1a over the whole register file: the first differing line is
            // the instruction *after* the one that diverged.
            let mut hash = 2_166_136_261u32;
            let mut mix = |byte: u8| {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(16_777_619);
            };
            for value in self
                .regs
                .gpr
                .iter()
                .copied()
                .chain([self.regs.mdr, self.regs.ram])
            {
                for byte in value.to_le_bytes() {
                    mix(byte);
                }
            }
            let mut gprs = String::new();
            for (index, value) in self.regs.gpr.iter().enumerate() {
                if index != 0 {
                    gprs.push(',');
                }
                gprs.push_str(&format!("{value:06X}"));
            }
            let _ = writeln!(
                f,
                "t={} ins={} {:04X}:{:02X} {:03X} a={:06X} p={:04X} dpr={:04X} ram={:06X} mdr={:06X} h={:08X} g={gprs}",
                DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed),
                self.ticks,
                self.regs.pb, self.regs.pc, opcode, self.regs.a, self.regs.p, self.regs.dpr,
                self.regs.ram, self.regs.mdr, hash
            );
        }
    }

    /// Debug: trace S-CPU writes into the CX4 windows (`CX4_TRACE=/path`).
    fn trace(&self, kind: &str, address: u16, data: u8) {
        let Some(path) = std::env::var_os("CX4_TRACE") else {
            return;
        };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "{} {kind} {address:04X} {data:02X}",
                DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
    }

    /// Debug: log writes to the CX4 bitmap cells that first diverge from ares.
    ///
    /// `CX4_WR_LOG=/path` enables it; `snes-info` appends `F <frame>` markers to
    /// the same file so the sequence can be compared against ares frame by frame.
    fn log_wrram(&self, address: u16, byte: u32) {
        let Some(path) = std::env::var_os("CX4_WR_LOG") else {
            return;
        };
        if !matches!(address, 0x71e | 0x71f | 0x7d0 | 0x7d1) {
            return;
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "W {:04X} byte={} ram={:06X} pb={:04X} pc={:02X} dpr={:04X} a={:06X}",
                address,
                byte,
                self.regs.ram,
                self.regs.pb,
                self.regs.pc,
                self.regs.dpr,
                self.regs.a
            );
        }
    }

    fn wrram_imm(&mut self, byte: u32, imm: u8) {
        let mut address = self.regs.dpr.wrapping_add(u32::from(imm)) as u16 & 0x0fff;
        if address >= 0xc00 {
            address -= 0x400;
        }
        let value = (self.regs.ram >> (byte * 8)) as u8;
        self.log_wrram(address, byte);
        self.data_ram[usize::from(address)] = value;
    }
}

/// Debug: frame counter kept current by the harness (`snes-info`) so the
/// instruction log can be limited to a frame range.
pub static DEBUG_FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Debug: first frame to include in the instruction log (`CX4_INSN_FROM`).
pub static DEBUG_INSN_FROM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Debug: S-CPU lead over the CX4 in CX4 cycles at the last sync (positive =
/// the coprocessor is behind), mirroring the ares `cc`/`pc` probe.
pub static DEBUG_CPU_LEAD: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Debug: master clock at the most recent bus access, so an instruction log can
/// be aligned event-for-event with a reference emulator's per-access log.
pub static DEBUG_CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: the live S-CPU program counter, kept current alongside `DEBUG_CLOCK`
/// so bus-write traces can name the instruction that performed the write.
pub static DEBUG_PC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: number of CX4 halt instructions since power-on.
pub static DEBUG_HALTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: number of CX4 jobs started by a `$7F4F` write.
pub static DEBUG_JOB_STARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: S-CPU stack pointer and the six bytes above it at a job request, so
/// the return address identifies which caller dispatched the job.
pub static DEBUG_SP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Debug: eight S-CPU stack bytes at the most recent job request.
pub static DEBUG_STACK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: coprocessor clock cycles charged since power-on (`CX4_SPIN_LOG`).
pub static DEBUG_CYCLES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Debug: iterations of the three `$7F5E` busy loops in the WRAM dispatcher.
pub static DEBUG_SPIN: [std::sync::atomic::AtomicU64; 3] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];
/// Whether the `$7F5E` busy-loop counter is enabled (cached once).
pub fn spin_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CX4_SPIN_LOG").is_some())
}

/// Debug: S-CPU instructions executed since power-on (`CX4_JOBS_LOG`).
pub static DEBUG_INSTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Whether the per-frame instruction counter is enabled (cached once).
pub fn instr_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CX4_JOBS_LOG").is_some())
}

/// Whether the `$88C6` state-machine entry log is enabled (cached once).
pub fn sm_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CX4_SM_LOG").is_some())
}

/// Whether the `$7F4F` job-request log is enabled (cached once).
pub fn job_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CX4_JOB_LOG").is_some())
}

/// Debug: hash of the whole S-CPU WRAM, sampled at a CX4 job start.
///
/// The S-CPU owns WRAM, so `system.rs` fills this in just before the `$7F4F`
/// write that starts a job; the job-start probe mixes it into its log line.
pub static DEBUG_WRAM_HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether the CX4 instruction log is enabled (cached once).
pub fn insn_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CX4_INSN_LOG").is_some())
}

/// Whether any debug log needs the master clock kept current (cached once).
pub fn debug_clock_needed() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("CX4_INSN_LOG").is_some()
            || std::env::var_os("CX4_TRACE").is_some()
            || std::env::var_os("CX4_SPIN_LOG").is_some()
    })
}

/// Cartesian-product helper used by [`cx4_window`] tests.
///
/// The S-CPU CX4 windows are fixed for all CX4 boards: registers at
/// `$00-$3F,$80-$BF:$6C00-$6FFF,$7C00-$7FFF` and data RAM at
/// `$00-$3F,$80-$BF:$6000-$6BFF,$7000-$7BFF`.
pub fn cx4_window(address: u32) -> Option<Cx4Window> {
    if Cx4::io_window(address).is_some() {
        Some(Cx4Window::Io)
    } else if Cx4::dram_window(address).is_some() {
        Some(Cx4Window::Dram)
    } else {
        None
    }
}

/// Which CX4 window an S-CPU address falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cx4Window {
    /// Register window (`$6C00`/`$7C00`).
    Io,
    /// Data RAM window (`$6000`/`$7000`).
    Dram,
}

impl Cx4 {
    /// Whether the cartridge header declares a CX4 (Hitachi) coprocessor.
    pub const fn header_has_cx4(cartridge_type: u8, mapping: Mapping) -> bool {
        matches!(cartridge_type, 0xf3) && matches!(mapping, Mapping::LoRom | Mapping::ExLoRom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmware() -> Vec<u8> {
        let mut bytes = vec![0u8; 1024 * 3];
        // Data-ROM word 0 = 0x123456 (little-endian 3 bytes).
        bytes[0] = 0x56;
        bytes[1] = 0x34;
        bytes[2] = 0x12;
        bytes
    }

    #[test]
    fn firmware_parses_the_data_rom() {
        let cx4 = Cx4::from_firmware(&firmware()).expect("firmware");
        assert_eq!(cx4.data_rom.len(), 1024);
        assert_eq!(cx4.data_rom[0], 0x12_3456);
        assert_eq!(cx4.program.len(), 512);
        assert_eq!(cx4.data_ram.len(), 0xC00);
    }

    #[test]
    fn firmware_rejects_the_wrong_size() {
        assert_eq!(
            Cx4::from_firmware(&[0; 3071]).unwrap_err(),
            Cx4Error::BadFirmwareSize {
                expected: 3072,
                found: 3071
            }
        );
    }

    #[test]
    fn powers_up_running_like_ares() {
        // ares `HG51B::power` is `r = {}; io = {}`, which leaves `halt` clear,
        // so the DSP executes from ROM address 0 until the S-CPU takes over.
        let cx4 = Cx4::from_firmware(&firmware()).expect("firmware");
        assert!(!cx4.io.halt);
        assert!(cx4.running());
        assert!(!cx4.io.cache.enable && !cx4.io.dma.enable);
    }

    #[test]
    fn classifies_the_two_s_cpu_windows() {
        // Register window: $00-$3F,$80-$BF:$6C00-$6FFF,$7C00-$7FFF.
        assert_eq!(Cx4::io_window(0x00_6C00), Some(0x000));
        assert_eq!(Cx4::io_window(0x00_7F40), Some(0x340));
        assert_eq!(Cx4::io_window(0xBF_7FFF), Some(0x3FF));
        assert_eq!(Cx4::io_window(0x40_6C00), None);
        // Data RAM: $00-$3F,$80-$BF:$6000-$6BFF,$7000-$7BFF.
        assert_eq!(Cx4::dram_window(0x00_6000), Some(0x000));
        assert_eq!(Cx4::dram_window(0x00_6BFF), Some(0xBFF));
        assert_eq!(Cx4::dram_window(0x00_7000), Some(0x000));
        assert_eq!(Cx4::dram_window(0x80_7BFF), Some(0xBFF));
        // $6C00-$6FFF is the register window, not data RAM.
        assert_eq!(Cx4::dram_window(0x00_6C00), None);
        assert_eq!(cx4_window(0x00_6C00), Some(Cx4Window::Io));
        assert_eq!(cx4_window(0x00_6000), Some(Cx4Window::Dram));
        assert_eq!(cx4_window(0x00_4000), None);
    }

    #[test]
    fn register_window_round_trips() {
        let mut cx4 = Cx4::from_firmware(&firmware()).expect("firmware");
        // GPR bytes at $7F80-$7F82 form one 24-bit register.
        cx4.write_io(0x7F80, 0x12);
        cx4.write_io(0x7F81, 0x34);
        cx4.write_io(0x7F82, 0x56);
        assert_eq!(cx4.regs.gpr[0], 0x56_3412);
        assert_eq!(cx4.read_io(0x7F80), 0x12);
        assert_eq!(cx4.read_io(0x7F81), 0x34);
        assert_eq!(cx4.read_io(0x7F82), 0x56);

        // Cache base, program bank, program counter and wait states.
        cx4.write_io(0x7F49, 0x00);
        cx4.write_io(0x7F4A, 0x80);
        cx4.write_io(0x7F4B, 0x02);
        assert_eq!(cx4.io.cache.base, 0x02_8000);
        assert_eq!(cx4.read_io(0x7F4B), 0x02);
        cx4.write_io(0x7F50, 0x44);
        assert_eq!((cx4.io.wait.rom, cx4.io.wait.ram), (4, 4));
        assert_eq!(cx4.read_io(0x7F50), 0x44);

        // Vector bytes.
        cx4.write_io(0x7F6A, 0xEF);
        assert_eq!(cx4.io.vector[0x0A], 0xEF);
        assert_eq!(cx4.read_io(0x7F6A), 0xEF);
    }

    #[test]
    fn data_ram_window_round_trips() {
        let mut cx4 = Cx4::from_firmware(&firmware()).expect("firmware");
        cx4.write_dram(0x000, 0xA5);
        cx4.write_dram(0xBFF, 0x5A);
        assert_eq!(cx4.read_dram(0x000), 0xA5);
        assert_eq!(cx4.read_dram(0xBFF), 0x5A);
        // Above 3 KiB the window is open bus and writes are dropped.
        assert_eq!(cx4.read_dram(0xC00), 0);
        cx4.write_dram(0xC00, 0xFF);
        assert_eq!(cx4.read_dram(0xC00), 0);
    }

    #[test]
    fn executes_a_cached_instruction() {
        // Zeroed ROM executes NOPs. The firmware powers up running, so the
        // first `run_one` fills the instruction cache and retires one opcode.
        let rom = vec![0u8; 0x1_0000];
        let mut cx4 = Cx4::from_firmware(&firmware()).expect("firmware");
        cx4.write_io(0x7F49, 0x00);
        cx4.write_io(0x7F4A, 0x80); // cache base = $008000
        cx4.write_io(0x7F4B, 0x00);
        let cost = cx4.run_one(&rom, &[]);
        assert!(cost > 0);
        assert_eq!(cx4.ticks, 1);
    }
}
