//! SA-1 (Super Accelerator 1) coprocessor.
//!
//! Ported from the ares `SA1` component. The SA-1 is a second WDC 65C816 with
//! its own 2 KiB internal RAM, a "super MMC" that banks the shared cartridge
//! ROM into both CPUs' address spaces, up to 256 KiB of shared battery RAM,
//! three DMA channels (including two character-conversion modes), three
//! timers, an arithmetic unit and a full set of interrupt/message registers.
//!
//! Both CPUs see the register block at `$2200-$23FF`, but only some registers
//! are readable/writable from each side. The S-CPU's ROM window is folded by
//! the super MMC; the SA-1 sees the ROM, BWRAM and IWRAM through its own map.

use crate::cpu::{Cpu, CpuBus};
use crate::rom::Region;

/// Size of the SA-1's internal work RAM (`$3000-$37FF` to both CPUs).
pub const IRAM_SIZE: usize = 0x800;

/// ares `Bus::mirror`: fold `address` into `size` bytes, handling
/// non-power-of-two sizes the same way the cartridge bus does.
pub fn mirror(mut address: u32, mut size: u32) -> u32 {
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

/// The SA-1 register file (`$2200-$23FF`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sa1Io {
    // $2200 CCNT
    /// SA-1 IRQ request.
    pub sa1_irq: bool,
    /// SA-1 bus request (`RDYB`).
    pub sa1_rdyb: bool,
    /// SA-1 reset (`RESB`, active low).
    pub sa1_resb: bool,
    /// SA-1 NMI request.
    pub sa1_nmi: bool,
    /// Message from the S-CPU to the SA-1.
    pub smeg: u8,
    // $2201 SIE / $2202 SIC
    /// S-CPU IRQ enable.
    pub cpu_irqen: bool,
    /// S-CPU character-DMA IRQ enable.
    pub chdma_irqen: bool,
    /// S-CPU IRQ clear.
    pub cpu_irqcl: bool,
    /// S-CPU character-DMA IRQ clear.
    pub chdma_irqcl: bool,
    /// `$2203/$2204` SA-1 reset vector.
    pub crv: u16,
    /// `$2205/$2206` SA-1 NMI vector.
    pub cnv: u16,
    /// `$2207/$2208` SA-1 IRQ vector.
    pub civ: u16,
    // $2209 SCNT
    /// S-CPU IRQ request.
    pub cpu_irq: bool,
    /// S-CPU IRQ vector switch.
    pub cpu_ivsw: bool,
    /// S-CPU NMI vector switch.
    pub cpu_nvsw: bool,
    /// Message from the SA-1 to the S-CPU.
    pub cmeg: u8,
    // $220a CIE / $220b CIC
    /// SA-1 IRQ enable.
    pub sa1_irqen: bool,
    /// SA-1 timer IRQ enable.
    pub timer_irqen: bool,
    /// SA-1 DMA IRQ enable.
    pub dma_irqen: bool,
    /// SA-1 NMI enable.
    pub sa1_nmien: bool,
    /// SA-1 IRQ clear.
    pub sa1_irqcl: bool,
    /// SA-1 timer IRQ clear.
    pub timer_irqcl: bool,
    /// SA-1 DMA IRQ clear.
    pub dma_irqcl: bool,
    /// SA-1 NMI clear.
    pub sa1_nmicl: bool,
    /// `$220c/$220d` S-CPU NMI vector.
    pub snv: u16,
    /// `$220e/$220f` S-CPU IRQ vector.
    pub siv: u16,
    // $2210 TMC
    /// Linear (1) versus H/V (0) timer select.
    pub hvselb: bool,
    /// Vertical timer enable.
    pub ven: bool,
    /// Horizontal timer enable.
    pub hen: bool,
    /// `$2212/$2213` H counter target (dots).
    pub hcnt: u16,
    /// `$2214/$2215` V counter target.
    pub vcnt: u16,
    // $2220-$2223 super MMC banks
    /// `$2220` C-bank mode (1 = banked).
    pub cbmode: bool,
    /// `$2220` C bank.
    pub cb: u32,
    /// `$2221` D-bank mode.
    pub dbmode: bool,
    /// `$2221` D bank.
    pub db: u32,
    /// `$2222` E-bank mode.
    pub ebmode: bool,
    /// `$2222` E bank.
    pub eb: u32,
    /// `$2223` F-bank mode.
    pub fbmode: bool,
    /// `$2223` F bank.
    pub fb: u32,
    // $2224-$222a BW-RAM mapping / protection
    /// `$2224` S-CPU BW-RAM bank.
    pub sbm: u8,
    /// `$2225` SA-1 BW-RAM mapping select (0 = 32x, 1 = 128x).
    pub sw46: bool,
    /// `$2225` SA-1 BW-RAM bank.
    pub cbm: u8,
    /// `$2226` S-CPU BW-RAM write enable.
    pub swen: bool,
    /// `$2227` SA-1 BW-RAM write enable.
    pub cwen: bool,
    /// `$2228` BW-RAM write-protected area.
    pub bwp: u8,
    /// `$2229` S-CPU I-RAM write protection.
    pub siwp: u8,
    /// `$222a` SA-1 I-RAM write protection.
    pub ciwp: u8,
    // $2230/$2231 DMA control
    /// DMA enable.
    pub dmaen: bool,
    /// DMA priority.
    pub dprio: bool,
    /// Character-conversion DMA enable.
    pub cden: bool,
    /// Character-conversion type select.
    pub cdsel: bool,
    /// Destination (0 = I-RAM, 1 = BW-RAM).
    pub dd: u8,
    /// Source (0 = ROM, 1 = BW-RAM, 2 = I-RAM).
    pub sd: u8,
    /// Character-conversion DMA has finished.
    pub chdend: bool,
    /// Character-conversion size.
    pub dmasize: u8,
    /// Character-conversion bit depth.
    pub dmacb: u8,
    /// `$2232-$2234` DMA source address.
    pub dsa: u32,
    /// `$2235-$2237` DMA destination address.
    pub dda: u32,
    /// `$2238/$2239` DMA terminal counter.
    pub dtc: u16,
    // $223f-224f bit-map
    /// `$223f` bit-map format (0 = 4bpp, 1 = 2bpp).
    pub bbf: bool,
    /// `$2240-$224f` bit-map register file.
    pub brf: [u8; 16],
    // $2250-$2254 arithmetic
    /// `$2250` accumulative multiplication.
    pub acm: bool,
    /// `$2250` multiply/divide select.
    pub md: bool,
    /// `$2251/$2252` multiplicand / dividend.
    pub ma: u16,
    /// `$2253/$2254` multiplier / divisor.
    pub mb: u16,
    // $2258-$225b variable-length bit processing
    /// `$2258` auto-increment mode.
    pub hl: bool,
    /// `$2258` bit count.
    pub vb: u8,
    /// `$2259-$225b` variable-length source address.
    pub va: u32,
    /// Current bit offset within `va`.
    pub vbit: u8,
    // $2300/$2301 flags
    /// S-CPU IRQ flag.
    pub cpu_irqfl: bool,
    /// S-CPU character-DMA IRQ flag.
    pub chdma_irqfl: bool,
    /// SA-1 IRQ flag.
    pub sa1_irqfl: bool,
    /// SA-1 timer IRQ flag.
    pub timer_irqfl: bool,
    /// SA-1 DMA IRQ flag.
    pub dma_irqfl: bool,
    /// SA-1 NMI flag.
    pub sa1_nmifl: bool,
    /// `$2302/$2303` latched H counter (dots).
    pub hcr: u16,
    /// `$2304/$2305` latched V counter.
    pub vcr: u16,
    /// `$2306-$230a` arithmetic result.
    pub mr: u64,
    /// `$230b` arithmetic overflow.
    pub overflow: bool,
}

impl Default for Sa1Io {
    fn default() -> Self {
        Self {
            sa1_irq: false,
            sa1_rdyb: false,
            sa1_resb: true,
            sa1_nmi: false,
            smeg: 0,
            cpu_irqen: false,
            chdma_irqen: false,
            cpu_irqcl: false,
            chdma_irqcl: false,
            crv: 0,
            cnv: 0,
            civ: 0,
            cpu_irq: false,
            cpu_ivsw: false,
            cpu_nvsw: false,
            cmeg: 0,
            sa1_irqen: false,
            timer_irqen: false,
            dma_irqen: false,
            sa1_nmien: false,
            sa1_irqcl: false,
            timer_irqcl: false,
            dma_irqcl: false,
            sa1_nmicl: false,
            snv: 0,
            siv: 0,
            hvselb: false,
            ven: false,
            hen: false,
            hcnt: 0,
            vcnt: 0,
            cbmode: false,
            cb: 0,
            dbmode: false,
            db: 1,
            ebmode: false,
            eb: 2,
            fbmode: false,
            fb: 3,
            sbm: 0,
            sw46: false,
            cbm: 0,
            swen: false,
            cwen: false,
            bwp: 0x0F,
            siwp: 0,
            ciwp: 0,
            dmaen: false,
            dprio: false,
            cden: false,
            cdsel: false,
            dd: 0,
            sd: 0,
            chdend: false,
            dmasize: 0,
            dmacb: 0,
            dsa: 0,
            dda: 0,
            dtc: 0,
            bbf: false,
            brf: [0; 16],
            acm: false,
            md: false,
            ma: 0,
            mb: 0,
            hl: false,
            vb: 16,
            va: 0,
            vbit: 0,
            cpu_irqfl: false,
            chdma_irqfl: false,
            sa1_irqfl: false,
            timer_irqfl: false,
            dma_irqfl: false,
            sa1_nmifl: false,
            hcr: 0,
            vcr: 0,
            mr: 0,
            overflow: false,
        }
    }
}

/// The SA-1 coprocessor: a second 65C816 plus the shared memory and registers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sa1 {
    /// The SA-1 CPU.
    pub cpu: Cpu,
    /// Registers.
    pub io: Sa1Io,
    /// 2 KiB internal RAM.
    pub iram: Vec<u8>,
    /// Pending interrupt; serviced at the next scheduling step.
    pub interrupt_pending: bool,
    /// Vector for the pending interrupt.
    pub vector: u16,
    /// Scanlines per frame (262 NTSC / 312 PAL).
    pub scanlines: u16,
    /// H counter in master clocks.
    pub hcounter: u16,
    /// V counter.
    pub vcounter: u16,
    /// Character-conversion DMA line.
    pub dma_line: u32,
    /// Whether an S-CPU BW-RAM read is a character-conversion read.
    pub bwram_dma: bool,
    /// Master clocks consumed by DMA/character conversion, charged to the
    /// SA-1's instruction credit by the scheduler.
    pub dma_clocks: u32,
    /// Last S-CPU address, for bus-conflict penalties.
    pub snes_mar: u32,
    /// S-CPU DRAM refresh active.
    pub snes_refresh: bool,
    /// Instructions retired (debug).
    pub ticks: u64,
    /// Address of the instruction currently executing (debug).
    #[serde(skip)]
    pub live_pc: u32,
    /// Address of the S-CPU instruction currently executing (debug).
    #[serde(skip)]
    pub snes_live_pc: u32,
    /// Master clock of the current S-CPU step (debug).
    #[serde(skip)]
    pub master_clock: u64,
    /// Normal DMA transfers executed (debug).
    pub dma_count: u64,
}

impl Sa1 {
    /// Create a powered-on SA-1.
    pub fn new(region: Region) -> Self {
        let mut sa1 = Self {
            cpu: Cpu::new(),
            io: Sa1Io::default(),
            iram: vec![0; IRAM_SIZE],
            interrupt_pending: false,
            vector: 0,
            scanlines: if region.is_pal() { 312 } else { 262 },
            hcounter: 0,
            vcounter: 0,
            dma_line: 0,
            bwram_dma: false,
            dma_clocks: 0,
            snes_mar: 0,
            snes_refresh: false,
            ticks: 0,
            live_pc: 0,
            snes_live_pc: 0,
            master_clock: 0,
            dma_count: 0,
        };
        sa1.power(region);
        sa1
    }

    /// Reset to the power-on state (ares `SA1::power`).
    pub fn power(&mut self, region: Region) {
        self.cpu = Cpu::new();
        self.io = Sa1Io::default();
        self.iram.fill(0);
        self.interrupt_pending = false;
        self.vector = 0;
        self.scanlines = if region.is_pal() { 312 } else { 262 };
        self.hcounter = 0;
        self.vcounter = 0;
        self.dma_line = 0;
        self.bwram_dma = false;
        self.dma_clocks = 0;
        self.snes_mar = 0;
        self.snes_refresh = false;
    }

    // --- Bus windows on the S-CPU side ------------------------------------

    /// `$00-$3F,$80-$BF:$2200-$23FF`.
    pub const fn in_io_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        let offset = address & 0xFFFF;
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x2200 && offset <= 0x23FF
    }

    /// `$00-$3F,$80-$BF:$3000-$37FF`.
    pub const fn in_iram_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        let offset = address & 0xFFFF;
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x3000 && offset <= 0x37FF
    }

    /// `$00-$3F,$80-$BF:$6000-$7FFF` or `$40-$4F:$0000-$FFFF`.
    pub const fn in_bwram_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        let offset = address & 0xFFFF;
        if bank >= 0x40 && bank <= 0x4F {
            return true;
        }
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && offset >= 0x6000 && offset <= 0x7FFF
    }

    /// `$00-$3F,$80-$BF:$8000-$FFFF` or `$C0-$FF:$0000-$FFFF`.
    pub const fn in_rom_window(address: u32) -> bool {
        let bank = (address >> 16) & 0xFF;
        if bank >= 0xC0 {
            return true;
        }
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && (address & 0xFFFF) >= 0x8000
    }

    // --- Memory helpers ---------------------------------------------------

    fn iram_read(&self, address: u32, data: u8) -> u8 {
        if self.iram.is_empty() {
            return data;
        }
        let index = mirror(address, IRAM_SIZE as u32) as usize;
        self.iram[index]
    }

    fn iram_write(&mut self, address: u32, data: u8) {
        if self.iram.is_empty() {
            return;
        }
        let index = mirror(address, IRAM_SIZE as u32) as usize;
        if std::env::var_os("SA1_LOG").is_some() && (0x220..=0x226).contains(&index) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/sa1.log")
            {
                let _ = writeln!(
                    f,
                    "I222 clock={} idx={:03X} val={:02X}",
                    self.master_clock, index, data
                );
            }
        }
        self.iram[index] = data;
    }

    fn log_bwram(side: &str, address: u32, data: u8, clock: u64) {
        if std::env::var_os("SA1_BWLOG").is_none() {
            return;
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/opencode/sa1bw.log")
        {
            let _ = writeln!(f, "{side} off={address:06X} val={data:02X} clock={clock}");
        }
    }

    fn bwram_read_raw(bwram: &[u8], address: u32, data: u8) -> u8 {
        if bwram.is_empty() {
            return data;
        }
        let index = mirror(address, bwram.len() as u32) as usize;
        bwram[index]
    }

    fn bwram_write_raw(bwram: &mut [u8], address: u32, data: u8) {
        if bwram.is_empty() {
            return;
        }
        let index = mirror(address, bwram.len() as u32) as usize;
        bwram[index] = data;
    }

    fn bwram_read_bitmap(&self, bwram: &[u8], address: u32, data: u8) -> u8 {
        if !self.io.bbf {
            let shift = address & 1;
            let byte = Self::bwram_read_raw(bwram, address >> 1, data);
            if shift == 0 {
                byte & 0x0F
            } else {
                byte >> 4
            }
        } else {
            let shift = address & 3;
            let byte = Self::bwram_read_raw(bwram, address >> 2, data);
            (byte >> (shift * 2)) & 0x03
        }
    }

    fn bwram_write_bitmap(&self, bwram: &mut [u8], address: u32, data: u8) {
        if !self.io.bbf {
            let shift = address & 1;
            let index = address >> 1;
            let old = Self::bwram_read_raw(bwram, index, 0);
            let value = if shift == 0 {
                (old & 0xF0) | (data & 0x0F)
            } else {
                (old & 0x0F) | ((data & 0x0F) << 4)
            };
            Self::bwram_write_raw(bwram, index, value);
        } else {
            let shift = address & 3;
            let index = address >> 2;
            let old = Self::bwram_read_raw(bwram, index, 0);
            let mask = !(0x03u8 << (shift * 2));
            let value = (old & mask) | ((data & 0x03) << (shift * 2));
            Self::bwram_write_raw(bwram, index, value);
        }
    }

    fn bwram_read_sa1(&self, bwram: &[u8], address: u32, data: u8) -> u8 {
        if !self.io.sw46 {
            let index = u32::from(self.io.cbm & 0x1F) * 0x2000 + (address & 0x1FFF);
            Self::bwram_read_raw(bwram, index, data)
        } else {
            let index = u32::from(self.io.cbm) * 0x2000 + (address & 0x1FFF);
            self.bwram_read_bitmap(bwram, index, data)
        }
    }

    fn bwram_write_sa1(&self, bwram: &mut [u8], address: u32, data: u8) {
        if !self.io.sw46 {
            let index = u32::from(self.io.cbm & 0x1F) * 0x2000 + (address & 0x1FFF);
            Self::bwram_write_raw(bwram, index, data);
        } else {
            let index = u32::from(self.io.cbm) * 0x2000 + (address & 0x1FFF);
            self.bwram_write_bitmap(bwram, index, data);
        }
    }

    /// Read the SA-1's view of its own address space (ares `SA1::read`).
    pub fn read_sa1(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) -> u8 {
        if address & 0x40_FE00 == 0x00_2200 {
            return self.read_io_sa1(address, rom, bwram, data);
        }
        if address & 0x40_8000 == 0x00_8000 || address & 0xC0_0000 == 0xC0_0000 {
            return self.rom_read_sa1(rom, address);
        }
        if address & 0x40_E000 == 0x00_6000
            || address & 0xE0_0000 == 0x40_0000
            || address & 0xF0_0000 == 0x60_0000
        {
            if address & 0x40_0000 != 0 && address & 0x20_0000 != 0 {
                return self.bwram_read_bitmap(bwram, address, data);
            }
            if address & 0x40_0000 != 0 {
                return Self::bwram_read_raw(bwram, address, data);
            }
            return self.bwram_read_sa1(bwram, address, data);
        }
        if address & 0x40_F800 == 0x00_0000 || address & 0x40_F800 == 0x00_3000 {
            return self.iram_read(address, data);
        }
        data
    }

    /// Write the SA-1's view of its own address space (ares `SA1::write`).
    pub fn write_sa1(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) {
        if address & 0x40_FE00 == 0x00_2200 {
            self.write_io_sa1(address, rom, bwram, data);
            return;
        }
        if address & 0x40_8000 == 0x00_8000 || address & 0xC0_0000 == 0xC0_0000 {
            return; // ROM is read-only
        }
        if address & 0x40_E000 == 0x00_6000
            || address & 0xE0_0000 == 0x40_0000
            || address & 0xF0_0000 == 0x60_0000
        {
            if address & 0x40_0000 != 0 && address & 0x20_0000 != 0 {
                self.bwram_write_bitmap(bwram, address, data);
            } else if address & 0x40_0000 != 0 {
                Self::bwram_write_raw(bwram, address, data);
            } else {
                Self::log_bwram(
                    "SA1",
                    mirror(address, bwram.len() as u32),
                    data,
                    self.master_clock,
                );
                self.bwram_write_sa1(bwram, address, data);
            }
            return;
        }
        if address & 0x40_F800 == 0x00_0000 || address & 0x40_F800 == 0x00_3000 {
            let protect = (address >> 8) & 0x07;
            if std::env::var_os("SA1_LOG").is_some() {
                let off = address & 0xFFFF;
                if (0x36D0..=0x3700).contains(&off) {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/opencode/sa1.log")
                    {
                        let _ = writeln!(
                            f,
                            "IW {:04X} <- {:02X} live={:06X} bs={:04X} bd={:04X} bdb={:02X} bp={:02X}",
                            off,
                            data,
                            self.live_pc,
                            self.cpu.s,
                            self.cpu.d,
                            self.cpu.dbr,
                            self.cpu.p()
                        );
                    }
                }
            }
            if self.io.ciwp & (1 << protect) != 0 {
                self.iram_write(address, data);
            }
        }
    }

    // --- ROM / BWRAM / IRAM on the S-CPU side -----------------------------

    fn rom_read_raw(rom: &[u8], address: u32) -> u8 {
        if rom.is_empty() {
            return 0;
        }
        rom[mirror(address, rom.len() as u32) as usize]
    }

    /// ares `SA1::ROM::readCPU`: super-MMC banking plus the S-CPU vector
    /// overrides at `$7FEA/$7FEB` and `$7FEE/$7FEF`.
    fn rom_read_cpu(&self, rom: &[u8], address: u32) -> u8 {
        if address & 0x00_FFE0 == 0x00_7FE0 {
            if address == 0x7FEA && self.io.cpu_nvsw {
                return self.io.snv as u8;
            }
            if address == 0x7FEB && self.io.cpu_nvsw {
                return (self.io.snv >> 8) as u8;
            }
            if address == 0x7FEE && self.io.cpu_ivsw {
                return self.io.siv as u8;
            }
            if address == 0x7FEF && self.io.cpu_ivsw {
                return (self.io.siv >> 8) as u8;
            }
        }

        let lo = address < 0x40_0000;
        let address = address & 0x3F_FFFF;
        let (mode, bank) = if address < 0x10_0000 {
            (self.io.cbmode, self.io.cb)
        } else if address < 0x20_0000 {
            (self.io.dbmode, self.io.db)
        } else if address < 0x30_0000 {
            (self.io.ebmode, self.io.eb)
        } else {
            (self.io.fbmode, self.io.fb)
        };

        if lo && !mode {
            Self::rom_read_raw(rom, address)
        } else {
            Self::rom_read_raw(rom, (bank << 20) | (address & 0x0F_FFFF))
        }
    }

    /// ares `SA1::ROM::readSA1`: fold the SA-1's LoROM window, then apply the
    /// super-MMC banking.
    fn rom_read_sa1(&self, rom: &[u8], address: u32) -> u8 {
        let mut address = address;
        if address & 0x40_8000 == 0x00_8000 {
            address =
                ((address & 0x80_0000) >> 2) | ((address & 0x3F_0000) >> 1) | (address & 0x00_7FFF);
        }
        self.rom_read_cpu(rom, address)
    }

    /// Read a ROM byte for the S-CPU (ares `SA1::ROM::readCPU` after the
    /// `bus.map` translation and mirroring).
    pub fn read_cpu_rom(&mut self, address: u32, rom: &[u8]) -> u8 {
        let translated = if address & 0x40_8000 == 0x00_8000 {
            ((address & 0x80_0000) >> 2) | ((address & 0x3F_0000) >> 1) | (address & 0x7FFF)
        } else {
            address
        };
        self.rom_read_cpu(rom, translated)
    }

    /// Read `$00-$3F,$80-$BF:$6000-$7FFF` or `$40-$4F:$0000-$FFFF` from the
    /// S-CPU (ares `SA1::BWRAM::readCPU` after the bus map).
    pub fn read_cpu_bwram(&mut self, address: u32, bwram: &mut [u8]) -> u8 {
        let mut offset = if address & 0x40_E000 == 0x00_6000 {
            address & 0x1FFF
        } else {
            mirror(address, bwram.len() as u32)
        };
        if offset < 0x2000 {
            offset = u32::from(self.io.sbm) * 0x2000 + (offset & 0x1FFF);
        }
        if self.bwram_dma {
            return self.dma_cc1_read(offset, bwram);
        }
        Self::bwram_read_raw(bwram, offset, 0)
    }

    /// Write `$00-$3F,$80-$BF:$6000-$7FFF` or `$40-$4F:$0000-$FFFF` from the
    /// S-CPU (ares `SA1::BWRAM::writeCPU`).
    pub fn write_cpu_bwram(&mut self, address: u32, bwram: &mut [u8], data: u8) {
        let mut offset = if address & 0x40_E000 == 0x00_6000 {
            address & 0x1FFF
        } else {
            mirror(address, bwram.len() as u32)
        };
        if offset < 0x2000 {
            offset = u32::from(self.io.sbm) * 0x2000 + (offset & 0x1FFF);
        }
        // BW-RAM protection only applies when both write enables are off.
        if !self.io.swen && !self.io.cwen && offset < (0x100u32 << self.io.bwp) {
            return;
        }
        Self::log_bwram("CPU", offset, data, self.master_clock);
        Self::bwram_write_raw(bwram, offset, data);
    }

    /// Read `$00-$3F,$80-$BF:$3000-$37FF` from the S-CPU.
    pub fn read_cpu_iram(&mut self, address: u32, data: u8) -> u8 {
        self.iram_read(address, data)
    }

    /// Write `$00-$3F,$80-$BF:$3000-$37FF` from the S-CPU.
    ///
    /// The ares bus map mirrors the address to `$0000-$07FF` before the write
    /// hook runs, so only `SIWP` bit 0 is consulted (ares quirk).
    pub fn write_cpu_iram(&mut self, address: u32, data: u8) {
        if std::env::var_os("SA1_LOG").is_some() {
            let index = (address as usize) & 0x7FF;
            if (0x200..=0x2FF).contains(&index) || (0x6D0..=0x700).contains(&index) {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/opencode/sa1.log")
                {
                    let _ = writeln!(
                        f,
                        "CI {:04X} <- {:02X} spc={:06X}",
                        index, data, self.snes_live_pc
                    );
                }
            }
        }
        if self.io.siwp & 1 != 0 {
            self.iram_write(address, data);
        }
    }

    // --- SA-1 CPU execution ----------------------------------------------

    /// Run one SA-1 step: an instruction, an interrupt, or an idle cycle.
    ///
    /// Returns the master clocks consumed (each SA-1 cycle is two).
    pub fn run_once(&mut self, rom: &[u8], bwram: &mut [u8]) -> u32 {
        let mut extra = 0u32;

        if self.io.sa1_rdyb || self.io.sa1_resb {
            // Held in reset or off the bus: one idle cycle, timers still run.
            self.timer_step();
            extra += 2;
        } else if self.interrupt_pending {
            self.interrupt_pending = false;
            let vector = self.vector;
            self.live_pc = self.cpu.pc_address();
            let mut cpu = std::mem::take(&mut self.cpu);
            let mut bus = Sa1Bus {
                sa1: self,
                rom,
                bwram,
            };
            let cost = cpu.service_hardware_vector(&mut bus, vector);
            self.cpu = cpu;
            extra += cost;
        } else {
            self.live_pc = self.cpu.pc_address();
            let mut cpu = std::mem::take(&mut self.cpu);
            let mut bus = Sa1Bus {
                sa1: self,
                rom,
                bwram,
            };
            let cost = cpu.step(&mut bus);
            self.cpu = cpu;
            extra += cost;
            self.last_cycle();
        }

        self.ticks = self.ticks.wrapping_add(1);
        extra + std::mem::take(&mut self.dma_clocks)
    }

    /// One 2-clock SA-1 cycle: advance the H/V or linear timer and test for a
    /// timer IRQ (ares `SA1::step`).
    fn timer_step(&mut self) {
        if !self.io.hvselb {
            self.hcounter = self.hcounter.wrapping_add(2);
            if self.hcounter >= 1364 {
                self.hcounter = 0;
                self.vcounter = self.vcounter.wrapping_add(1);
                if self.vcounter >= self.scanlines {
                    self.vcounter = 0;
                }
            }
        } else {
            self.hcounter = self.hcounter.wrapping_add(2);
            self.vcounter = self.vcounter.wrapping_add(self.hcounter >> 11);
            self.hcounter &= 0x07FF;
            self.vcounter &= 0x01FF;
        }

        let mode = u8::from(self.io.hen) | (u8::from(self.io.ven) << 1);
        match mode {
            1 if self.hcounter == self.io.hcnt.wrapping_shl(2) => self.trigger_irq(),
            2 if self.vcounter == self.io.vcnt && self.hcounter == 0 => self.trigger_irq(),
            3 if self.vcounter == self.io.vcnt && self.hcounter == self.io.hcnt.wrapping_shl(2) => {
                self.trigger_irq()
            }
            _ => {}
        }
    }

    fn trigger_irq(&mut self) {
        self.io.timer_irqfl = true;
        if self.io.timer_irqen {
            self.io.timer_irqcl = false;
        }
    }

    /// ares `SA1::lastCycle`: latch a pending interrupt at an instruction end.
    fn last_cycle(&mut self) {
        if self.io.sa1_nmi && !self.io.sa1_nmicl {
            self.interrupt_pending = true;
            self.vector = self.io.cnv;
            self.io.sa1_nmifl = true;
            self.io.sa1_nmicl = true;
            self.cpu.wai = false;
            return;
        }
        if self.cpu.flags.irq_disable {
            return;
        }
        if self.io.timer_irqen && !self.io.timer_irqcl {
            self.interrupt_pending = true;
            self.vector = self.io.civ;
            self.io.timer_irqfl = true;
            self.cpu.wai = false;
        } else if self.io.dma_irqen && !self.io.dma_irqcl {
            self.interrupt_pending = true;
            self.vector = self.io.civ;
            self.io.dma_irqfl = true;
            self.cpu.wai = false;
        } else if self.io.sa1_irq && !self.io.sa1_irqcl {
            self.interrupt_pending = true;
            self.vector = self.io.civ;
            self.io.sa1_irqfl = true;
            self.cpu.wai = false;
        }
    }

    /// Bus-conflict steps for one SA-1 access (ares `*::conflict`).
    fn access_steps(&self, address: u32) -> u32 {
        let rom_conflict = |mar: u32| (mar & 0x408000) == 0x008000 || (mar & 0xC00000) == 0xC00000;
        let bwram_conflict =
            |mar: u32| (mar & 0x40E000) == 0x006000 || (mar & 0xF00000) == 0x400000;
        let iram_conflict = |mar: u32| (mar & 0x40F800) == 0x003000 && self.snes_refresh;

        if address & 0x40_FE00 == 0x00_2200 {
            return 1;
        }
        if address & 0x40_8000 == 0x00_8000 || address & 0xC0_0000 == 0xC0_0000 {
            return 1 + u32::from(rom_conflict(self.snes_mar));
        }
        if address & 0x40_E000 == 0x00_6000
            || address & 0xE0_0000 == 0x40_0000
            || address & 0xF0_0000 == 0x60_0000
        {
            return 2
                + u32::from(bwram_conflict(self.snes_mar))
                + u32::from(bwram_conflict(self.snes_mar));
        }
        if address & 0x40_F800 == 0x00_0000 || address & 0x40_F800 == 0x00_3000 {
            return 1
                + u32::from(iram_conflict(self.snes_mar))
                + u32::from(iram_conflict(self.snes_mar));
        }
        1
    }

    // --- DMA --------------------------------------------------------------

    /// ares `SA1::dmaNormal`.
    fn dma_normal(&mut self, rom: &[u8], bwram: &mut [u8]) {
        self.dma_count = self.dma_count.wrapping_add(1);
        let rom_conflict =
            (self.snes_mar & 0x408000) == 0x008000 || (self.snes_mar & 0xC00000) == 0xC00000;
        let bwram_conflict =
            (self.snes_mar & 0x40E000) == 0x006000 || (self.snes_mar & 0xF00000) == 0x400000;
        let iram_conflict = (self.snes_mar & 0x40F800) == 0x003000 && self.snes_refresh;

        let count = self.io.dtc;
        for _ in 0..count {
            let source = self.io.dsa;
            let target = self.io.dda;
            self.io.dsa = (self.io.dsa + 1) & 0x00FF_FFFF;
            self.io.dda = (self.io.dda + 1) & 0x00FF_FFFF;
            let mut data = self.cpu.mdr;

            match (self.io.sd, self.io.dd) {
                (0, 1) => {
                    self.dma_clocks +=
                        4 + u32::from(bwram_conflict) * 2 + u32::from(bwram_conflict) * 2;
                    data = self.rom_read_sa1(rom, source);
                    Self::bwram_write_raw(bwram, target, data);
                }
                (0, 0) => {
                    self.dma_clocks += 2
                        + u32::from(iram_conflict || rom_conflict) * 2
                        + u32::from(iram_conflict) * 2;
                    data = self.rom_read_sa1(rom, source);
                    self.iram_write(target, data);
                }
                (1, 0) => {
                    self.dma_clocks += 4
                        + u32::from(bwram_conflict || iram_conflict) * 2
                        + u32::from(bwram_conflict) * 2;
                    data = Self::bwram_read_raw(bwram, source, data);
                    self.iram_write(target, data);
                }
                (2, 1) => {
                    self.dma_clocks += 4
                        + u32::from(bwram_conflict || iram_conflict) * 2
                        + u32::from(bwram_conflict) * 2;
                    data = self.iram_read(source, data);
                    Self::bwram_write_raw(bwram, target, data);
                }
                _ => {}
            }
        }
        self.io.dtc = 0xFFFF;
        self.io.dma_irqfl = true;
        if self.io.dma_irqen {
            self.io.dma_irqcl = false;
        }
    }

    /// ares `SA1::dmaCC1`.
    fn dma_cc1(&mut self) {
        self.bwram_dma = true;
        self.io.chdma_irqfl = true;
        if self.io.chdma_irqen {
            self.io.chdma_irqcl = false;
        }
    }

    /// ares `SA1::dmaCC1Read`: buffer the next character into I-RAM and return
    /// the requested byte.
    fn dma_cc1_read(&mut self, address: u32, bwram: &mut [u8]) -> u8 {
        let charmask = (1u32 << (6 - self.io.dmacb)) - 1;
        if address & charmask == 0 {
            let bpp = 2u32 << (2 - self.io.dmacb);
            let bpl = (8u32 << self.io.dmasize) >> self.io.dmacb;
            let bwmask = (bwram.len() as u32).wrapping_sub(1);
            let tile = ((address.wrapping_sub(self.io.dsa)) & bwmask) >> (6 - self.io.dmacb);
            let ty = tile >> self.io.dmasize;
            let tx = tile & ((1u32 << self.io.dmasize) - 1);
            let mut bwaddr = self.io.dsa.wrapping_add(ty * 8 * bpl + tx * bpp);

            for y in 0..8u32 {
                let mut data = 0u64;
                for byte in 0..bpp {
                    let value = Self::bwram_read_raw(bwram, (bwaddr + byte) & bwmask, 0);
                    data |= u64::from(value) << (byte << 3);
                }
                bwaddr = bwaddr.wrapping_add(bpl);

                let mut out = [0u8; 8];
                for x in 0..8u32 {
                    out[0] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    out[1] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    if self.io.dmacb == 2 {
                        continue;
                    }
                    out[2] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    out[3] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    if self.io.dmacb == 1 {
                        continue;
                    }
                    out[4] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    out[5] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    out[6] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                    out[7] |= ((data & 1) as u8) << (7 - x);
                    data >>= 1;
                }

                for byte in 0..bpp {
                    let p = self
                        .io
                        .dda
                        .wrapping_add((y << 1) + ((byte & 6) << 3) + (byte & 1));
                    self.iram_write(p & 0x07FF, out[byte as usize]);
                }
            }
        }

        self.iram_read((self.io.dda.wrapping_add(address & charmask)) & 0x07FF, 0)
    }

    /// ares `SA1::dmaCC2`: convert one line of bitmap data from the register
    /// file into I-RAM.
    fn dma_cc2(&mut self) {
        let base = ((self.dma_line & 1) << 3) as usize;
        let brf = [
            self.io.brf[base],
            self.io.brf[base + 1],
            self.io.brf[base + 2],
            self.io.brf[base + 3],
            self.io.brf[base + 4],
            self.io.brf[base + 5],
            self.io.brf[base + 6],
            self.io.brf[base + 7],
        ];
        let bpp = 2u32 << (2 - self.io.dmacb);
        let mut address = self.io.dda & 0x07FF;
        address &= !((1u32 << (7 - self.io.dmacb)) - 1);
        address = address
            .wrapping_add((self.dma_line & 8) * bpp)
            .wrapping_add((self.dma_line & 7) * 2);

        for byte in 0..bpp {
            let mut output = 0u8;
            for bit in 0..8u32 {
                output |= ((brf[bit as usize] >> byte) & 1) << (7 - bit);
            }
            self.iram_write(address + ((byte & 6) << 3) + (byte & 1), output);
        }

        self.dma_line = (self.dma_line + 1) & 15;
    }

    // --- Registers --------------------------------------------------------

    /// ares `SA1::readIOCPU`: the S-CPU side of `$2200-$23FF`.
    pub fn read_cpu_io(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) -> u8 {
        let _ = (rom, bwram);
        match 0x2200 | (address & 0x01FF) {
            0x2300 => {
                (self.io.cmeg & 0x0F)
                    | (u8::from(self.io.cpu_nvsw) << 4)
                    | (u8::from(self.io.chdma_irqfl) << 5)
                    | (u8::from(self.io.cpu_ivsw) << 6)
                    | (u8::from(self.io.cpu_irqfl) << 7)
            }
            _ => data,
        }
    }

    /// ares `SA1::writeIOCPU`: the S-CPU side of `$2200-$23FF`.
    pub fn write_cpu_io(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) {
        if std::env::var_os("SA1_LOG").is_some() && (address & 0x1FF) == 0x000 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/sa1.log")
            {
                let _ = writeln!(
                    f,
                    "CCNT clock={} data={:02X} old_resb={} old_rdyb={}",
                    self.master_clock, data, self.io.sa1_resb, self.io.sa1_rdyb
                );
            }
        }
        match 0x2200 | (address & 0x01FF) {
            0x2200 => {
                if self.io.sa1_resb && data & 0x20 == 0 {
                    // Reset the SA-1 CPU: start at CRV with bank 0.
                    self.cpu.pc = self.io.crv;
                    self.cpu.pbr = 0;
                    self.cpu.dbr = 0;
                    self.cpu.stp = false;
                    self.io.ciwp = 0;
                }
                self.io.smeg = data & 0x0F;
                self.io.sa1_nmi = data & 0x10 != 0;
                self.io.sa1_resb = data & 0x20 != 0;
                self.io.sa1_rdyb = data & 0x40 != 0;
                self.io.sa1_irq = data & 0x80 != 0;
                if self.io.sa1_irq {
                    self.io.sa1_irqfl = true;
                    if self.io.sa1_irqen {
                        self.io.sa1_irqcl = false;
                    }
                }
                if self.io.sa1_nmi {
                    self.io.sa1_nmifl = true;
                    if self.io.sa1_nmien {
                        self.io.sa1_nmicl = false;
                    }
                }
            }
            0x2201 => {
                if !self.io.chdma_irqen && data & 0x20 != 0 && self.io.chdma_irqfl {
                    self.io.chdma_irqcl = false;
                }
                if !self.io.cpu_irqen && data & 0x80 != 0 && self.io.cpu_irqfl {
                    self.io.cpu_irqcl = false;
                }
                self.io.chdma_irqen = data & 0x20 != 0;
                self.io.cpu_irqen = data & 0x80 != 0;
            }
            0x2202 => {
                self.io.chdma_irqcl = data & 0x20 != 0;
                self.io.cpu_irqcl = data & 0x80 != 0;
                if self.io.chdma_irqcl {
                    self.io.chdma_irqfl = false;
                }
                if self.io.cpu_irqcl {
                    self.io.cpu_irqfl = false;
                }
            }
            0x2203 => self.io.crv = (self.io.crv & 0xFF00) | u16::from(data),
            0x2204 => self.io.crv = (self.io.crv & 0x00FF) | (u16::from(data) << 8),
            0x2205 => self.io.cnv = (self.io.cnv & 0xFF00) | u16::from(data),
            0x2206 => self.io.cnv = (self.io.cnv & 0x00FF) | (u16::from(data) << 8),
            0x2207 => self.io.civ = (self.io.civ & 0xFF00) | u16::from(data),
            0x2208 => self.io.civ = (self.io.civ & 0x00FF) | (u16::from(data) << 8),
            0x2220 => {
                self.io.cb = u32::from(data & 0x07);
                self.io.cbmode = data & 0x80 != 0;
            }
            0x2221 => {
                self.io.db = u32::from(data & 0x07);
                self.io.dbmode = data & 0x80 != 0;
            }
            0x2222 => {
                self.io.eb = u32::from(data & 0x07);
                self.io.ebmode = data & 0x80 != 0;
            }
            0x2223 => {
                self.io.fb = u32::from(data & 0x07);
                self.io.fbmode = data & 0x80 != 0;
            }
            0x2224 => self.io.sbm = data & 0x1F,
            0x2226 => self.io.swen = data & 0x80 != 0,
            0x2228 => self.io.bwp = data & 0x0F,
            0x2229 => self.io.siwp = data,
            0x2231..=0x2237 => self.write_io_shared(address, rom, bwram, data),
            _ => {}
        }
    }

    /// ares `SA1::readIOSA1`: the SA-1 side of `$2200-$23FF`.
    pub fn read_io_sa1(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) -> u8 {
        match 0x2200 | (address & 0x01FF) {
            0x2301 => {
                (self.io.smeg & 0x0F)
                    | (u8::from(self.io.sa1_nmifl) << 4)
                    | (u8::from(self.io.dma_irqfl) << 5)
                    | (u8::from(self.io.timer_irqfl) << 6)
                    | (u8::from(self.io.sa1_irqfl) << 7)
            }
            0x2302 => {
                self.io.hcr = self.hcounter >> 2;
                self.io.vcr = self.vcounter;
                self.io.hcr as u8
            }
            0x2303 => (self.io.hcr >> 8) as u8,
            0x2304 => self.io.vcr as u8,
            0x2305 => (self.io.vcr >> 8) as u8,
            0x2306 => self.io.mr as u8,
            0x2307 => (self.io.mr >> 8) as u8,
            0x2308 => (self.io.mr >> 16) as u8,
            0x2309 => (self.io.mr >> 24) as u8,
            0x230A => (self.io.mr >> 32) as u8,
            0x230B => u8::from(self.io.overflow) << 7,
            0x230C => {
                let value = self.read_vbr(rom, bwram, self.io.va) as u32
                    | ((self.read_vbr(rom, bwram, self.io.va.wrapping_add(1)) as u32) << 8)
                    | ((self.read_vbr(rom, bwram, self.io.va.wrapping_add(2)) as u32) << 16);
                (value >> self.io.vbit) as u8
            }
            0x230D => {
                let value = self.read_vbr(rom, bwram, self.io.va) as u32
                    | ((self.read_vbr(rom, bwram, self.io.va.wrapping_add(1)) as u32) << 8)
                    | ((self.read_vbr(rom, bwram, self.io.va.wrapping_add(2)) as u32) << 16);
                let value = value >> self.io.vbit;
                if self.io.hl {
                    self.io.vbit += self.io.vb;
                    self.io.va =
                        self.io.va.wrapping_add(u32::from(self.io.vbit >> 3)) & 0x00FF_FFFF;
                    self.io.vbit &= 7;
                }
                (value >> 8) as u8
            }
            _ => data,
        }
    }

    /// ares `SA1::readVBR`: variable-length reads bypass the MMIO window.
    fn read_vbr(&self, rom: &[u8], bwram: &mut [u8], address: u32) -> u8 {
        if address & 0x40_8000 == 0x00_8000 || address & 0xC0_0000 == 0xC0_0000 {
            return Self::rom_read_raw(rom, address);
        }
        if address & 0x40_E000 == 0x00_6000 || address & 0xF0_0000 == 0x40_0000 {
            return Self::bwram_read_raw(bwram, address, 0);
        }
        if address & 0x40_F800 == 0x00_0000 || address & 0x40_F800 == 0x00_3000 {
            return self.iram_read(address, 0);
        }
        0xFF
    }

    /// ares `SA1::writeIOSA1`: the SA-1 side of `$2200-$23FF`.
    pub fn write_io_sa1(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) {
        match 0x2200 | (address & 0x01FF) {
            0x2209 => {
                self.io.cmeg = data & 0x0F;
                self.io.cpu_nvsw = data & 0x10 != 0;
                self.io.cpu_ivsw = data & 0x40 != 0;
                self.io.cpu_irq = data & 0x80 != 0;
                if self.io.cpu_irq {
                    self.io.cpu_irqfl = true;
                    if self.io.cpu_irqen {
                        self.io.cpu_irqcl = false;
                    }
                }
            }
            0x220A => {
                if !self.io.sa1_nmien && data & 0x10 != 0 && self.io.sa1_nmifl {
                    self.io.sa1_nmicl = false;
                }
                if !self.io.dma_irqen && data & 0x20 != 0 && self.io.dma_irqfl {
                    self.io.dma_irqcl = false;
                }
                if !self.io.timer_irqen && data & 0x40 != 0 && self.io.timer_irqfl {
                    self.io.timer_irqcl = false;
                }
                if !self.io.sa1_irqen && data & 0x80 != 0 && self.io.sa1_irqfl {
                    self.io.sa1_irqcl = false;
                }
                self.io.sa1_nmien = data & 0x10 != 0;
                self.io.dma_irqen = data & 0x20 != 0;
                self.io.timer_irqen = data & 0x40 != 0;
                self.io.sa1_irqen = data & 0x80 != 0;
            }
            0x220B => {
                self.io.sa1_nmicl = data & 0x10 != 0;
                self.io.dma_irqcl = data & 0x20 != 0;
                self.io.timer_irqcl = data & 0x40 != 0;
                self.io.sa1_irqcl = data & 0x80 != 0;
                if self.io.sa1_nmicl {
                    self.io.sa1_nmifl = false;
                }
                if self.io.sa1_irqcl {
                    self.io.sa1_irqfl = false;
                }
                if self.io.timer_irqcl {
                    self.io.timer_irqfl = false;
                }
                if self.io.dma_irqcl {
                    self.io.dma_irqfl = false;
                }
            }
            0x220C => self.io.snv = (self.io.snv & 0xFF00) | u16::from(data),
            0x220D => self.io.snv = (self.io.snv & 0x00FF) | (u16::from(data) << 8),
            0x220E => self.io.siv = (self.io.siv & 0xFF00) | u16::from(data),
            0x220F => self.io.siv = (self.io.siv & 0x00FF) | (u16::from(data) << 8),
            0x2210 => {
                self.io.hen = data & 0x01 != 0;
                self.io.ven = data & 0x02 != 0;
                self.io.hvselb = data & 0x80 != 0;
            }
            0x2211 => {
                self.vcounter = 0;
                self.hcounter = 0;
            }
            0x2212 => self.io.hcnt = (self.io.hcnt & 0xFF00) | u16::from(data),
            0x2213 => self.io.hcnt = (self.io.hcnt & 0x00FF) | (u16::from(data) << 8),
            0x2214 => self.io.vcnt = (self.io.vcnt & 0xFF00) | u16::from(data),
            0x2215 => self.io.vcnt = (self.io.vcnt & 0x00FF) | (u16::from(data) << 8),
            0x2225 => {
                self.io.cbm = data & 0x7F;
                self.io.sw46 = data & 0x80 != 0;
            }
            0x2227 => self.io.cwen = data & 0x80 != 0,
            0x222A => self.io.ciwp = data,
            0x2230 => {
                self.io.sd = data & 0x03;
                self.io.dd = (data >> 2) & 0x01;
                self.io.cdsel = data & 0x10 != 0;
                self.io.cden = data & 0x20 != 0;
                self.io.dprio = data & 0x40 != 0;
                self.io.dmaen = data & 0x80 != 0;
                if !self.io.dmaen {
                    self.dma_line = 0;
                }
            }
            0x2231..=0x2237 => self.write_io_shared(address, rom, bwram, data),
            0x2238 => self.io.dtc = (self.io.dtc & 0xFF00) | u16::from(data),
            0x2239 => self.io.dtc = (self.io.dtc & 0x00FF) | (u16::from(data) << 8),
            0x223F => self.io.bbf = data & 0x80 != 0,
            0x2240..=0x224F => {
                let index = (address & 0x0F) as usize;
                self.io.brf[index] = data;
                if (index == 7 || index == 15) && self.io.dmaen && self.io.cden && !self.io.cdsel {
                    self.dma_cc2();
                }
            }
            0x2250 => {
                self.io.md = data & 0x01 != 0;
                self.io.acm = data & 0x02 != 0;
                if self.io.acm {
                    self.io.mr = 0;
                }
            }
            0x2251 => self.io.ma = (self.io.ma & 0xFF00) | u16::from(data),
            0x2252 => self.io.ma = (self.io.ma & 0x00FF) | (u16::from(data) << 8),
            0x2253 => self.io.mb = (self.io.mb & 0xFF00) | u16::from(data),
            0x2254 => {
                self.io.mb = (self.io.mb & 0x00FF) | (u16::from(data) << 8);
                self.arithmetic();
            }
            0x2258 => {
                self.io.vb = data & 0x0F;
                self.io.hl = data & 0x80 != 0;
                if self.io.vb == 0 {
                    self.io.vb = 16;
                }
                if !self.io.hl {
                    self.io.vbit = self.io.vbit.wrapping_add(self.io.vb);
                    self.io.va =
                        self.io.va.wrapping_add(u32::from(self.io.vbit >> 3)) & 0x00FF_FFFF;
                    self.io.vbit &= 7;
                }
            }
            0x2259 => self.io.va = (self.io.va & 0xFFFF00) | u32::from(data),
            0x225A => self.io.va = (self.io.va & 0xFF00FF) | (u32::from(data) << 8),
            0x225B => {
                self.io.va = (self.io.va & 0x00FFFF) | (u32::from(data) << 16);
                self.io.vbit = 0;
            }
            _ => {}
        }
    }

    fn arithmetic(&mut self) {
        if !self.io.acm {
            if !self.io.md {
                // Signed multiplication.
                let product = i32::from(self.io.ma as i16) * i32::from(self.io.mb as i16);
                self.io.mr = u64::from(product as u32);
                self.io.mb = 0;
            } else {
                if self.io.mb == 0 {
                    self.io.mr = 0;
                } else {
                    let dividend = i32::from(self.io.ma as i16);
                    let divisor = i32::from(self.io.mb);
                    let remainder = if dividend >= 0 {
                        dividend % divisor
                    } else {
                        (dividend % divisor + divisor) % divisor
                    };
                    let quotient = (dividend - remainder) / divisor;
                    self.io.mr = (u64::from(remainder as u16) << 16) | u64::from(quotient as u16);
                }
                self.io.ma = 0;
                self.io.mb = 0;
            }
        } else {
            let product = i64::from(self.io.ma as i16) * i64::from(self.io.mb as i16);
            self.io.mr = self.io.mr.wrapping_add(product as u64);
            self.io.overflow = self.io.mr >> 40 != 0;
            self.io.mr &= 0xFF_FFFF_FFFF;
            self.io.mb = 0;
        }
    }

    /// ares `SA1::writeIOShared`: the registers both CPUs can write (DMA).
    fn write_io_shared(&mut self, address: u32, rom: &[u8], bwram: &mut [u8], data: u8) {
        match 0x2200 | (address & 0x01FF) {
            0x2231 => {
                self.io.dmacb = data & 0x03;
                self.io.dmasize = (data >> 2) & 0x07;
                self.io.chdend = data & 0x80 != 0;
                if self.io.dmacb > 2 {
                    self.io.dmacb = 2;
                }
                if self.io.dmasize > 5 {
                    self.io.dmasize = 5;
                }
                if self.io.chdend {
                    self.bwram_dma = false;
                }
            }
            0x2232 => self.io.dsa = (self.io.dsa & 0xFFFF00) | u32::from(data),
            0x2233 => self.io.dsa = (self.io.dsa & 0xFF00FF) | (u32::from(data) << 8),
            0x2234 => self.io.dsa = (self.io.dsa & 0x00FFFF) | (u32::from(data) << 16),
            0x2235 => self.io.dda = (self.io.dda & 0xFFFF00) | u32::from(data),
            0x2236 => {
                self.io.dda = (self.io.dda & 0xFF00FF) | (u32::from(data) << 8);
                if self.io.dmaen {
                    if !self.io.cden && self.io.dd == 0 {
                        self.dma_normal(rom, bwram);
                    } else if self.io.cden && self.io.cdsel {
                        self.dma_cc1();
                    }
                }
            }
            0x2237 => {
                self.io.dda = (self.io.dda & 0x00FFFF) | (u32::from(data) << 16);
                if self.io.dmaen && !self.io.cden && self.io.dd == 1 {
                    self.dma_normal(rom, bwram);
                }
            }
            _ => {}
        }
    }

    /// The S-CPU IRQ level requested by the SA-1 (ares `cpu.irq(1)`/`irq(0)`).
    pub const fn cpu_irq_line(&self) -> bool {
        (self.io.cpu_irqfl && self.io.cpu_irqen) || (self.io.chdma_irqfl && self.io.chdma_irqen)
    }

    /// Whether an S-CPU BW-RAM read must be served by character conversion.
    pub const fn bwram_dma_active(&self) -> bool {
        self.bwram_dma
    }
}

/// The SA-1 CPU's view of the bus.
struct Sa1Bus<'a> {
    sa1: &'a mut Sa1,
    rom: &'a [u8],
    bwram: &'a mut [u8],
}

impl CpuBus for Sa1Bus<'_> {
    fn read(&mut self, address: u32, mdr: u8) -> u8 {
        self.sa1.read_sa1(address, self.rom, self.bwram, mdr)
    }

    fn write(&mut self, address: u32, value: u8) {
        self.sa1.write_sa1(address, self.rom, self.bwram, value)
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        let steps = self.sa1.access_steps(address);
        for _ in 0..steps {
            self.sa1.timer_step();
        }
        steps * 2
    }

    fn idle_cycles(&self) -> u32 {
        2
    }

    fn idle(&mut self) {
        self.sa1.timer_step();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x40_0000];
        // Distinct byte per 1 MiB bank, at the start of each.
        rom[0x00_0000] = 0x10;
        rom[0x10_0000] = 0x11;
        rom[0x20_0000] = 0x12;
        rom[0x30_0000] = 0x13;
        rom[0x00_8000] = 0x20;
        rom[0x10_8000] = 0x21;
        rom[0x20_8000] = 0x22;
        rom[0x30_8000] = 0x23;
        rom
    }

    #[test]
    fn mirror_matches_the_cartridge_bus() {
        assert_eq!(mirror(0x0000, 0x8000), 0x0000);
        assert_eq!(mirror(0x8000, 0x8000), 0x0000);
        assert_eq!(mirror(0x8001, 0x8000), 0x0001);
        assert_eq!(mirror(0x3000, 0x800), 0x0000);
        assert_eq!(mirror(0x37FF, 0x800), 0x07FF);
        // Non-power-of-two sizes fold like ares: the high bit above the size
        // becomes the base instead of being dropped.
        assert_eq!(mirror(0x600000, 0x600000), 0x400000);
        assert_eq!(mirror(0x400000, 0x400000), 0x000000);
    }

    #[test]
    fn super_mmc_banks_the_s_cpu_rom_window() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        // LoROM: bank 0's $8000 window starts at ROM offset 0.
        assert_eq!(sa1.read_cpu_rom(0x00_8000, &rom), 0x10);
        assert_eq!(sa1.read_cpu_rom(0x80_8000, &rom), 0x12);

        // C/D/E/F default to banks 0,1,2,3.
        assert_eq!(sa1.read_cpu_rom(0xC0_0000, &rom), 0x10);
        assert_eq!(sa1.read_cpu_rom(0xD0_0000, &rom), 0x11);
        assert_eq!(sa1.read_cpu_rom(0xE0_0000, &rom), 0x12);
        assert_eq!(sa1.read_cpu_rom(0xF0_0000, &rom), 0x13);

        // Bank registers remap the $C0-$FF window.
        sa1.write_cpu_io(0x2220, &rom, &mut [], 0x03);
        assert_eq!(sa1.read_cpu_rom(0xC0_0000, &rom), 0x13);

        // For the $00-$3F window the mode bit selects direct versus banked.
        sa1.write_cpu_io(0x2221, &rom, &mut [], 0x82); // D: bank 2, mode 1
        assert_eq!(sa1.read_cpu_rom(0x20_8000, &rom), 0x12);
        sa1.write_cpu_io(0x2221, &rom, &mut [], 0x8A); // mode 1: banked
        assert_eq!(sa1.read_cpu_rom(0x20_8000, &rom), 0x12);
    }

    #[test]
    fn sa1_side_folds_the_lorom_window() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];
        assert_eq!(sa1.read_sa1(0x00_8000, &rom, &mut bwram, 0), 0x10);
        assert_eq!(sa1.read_sa1(0x80_8000, &rom, &mut bwram, 0), 0x12);
        assert_eq!(sa1.read_sa1(0x20_8000, &rom, &mut bwram, 0), 0x11);
    }

    #[test]
    fn bwram_bitmap_reads_and_writes() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];
        sa1.io.bbf = false; // 4bpp
        sa1.bwram_write_bitmap(&mut bwram, 0, 0x0A);
        sa1.bwram_write_bitmap(&mut bwram, 1, 0x05);
        assert_eq!(bwram[0], 0x5A);
        assert_eq!(sa1.bwram_read_bitmap(&bwram, 0, 0), 0x0A);
        assert_eq!(sa1.bwram_read_bitmap(&bwram, 1, 0), 0x05);

        let mut bwram = vec![0u8; 0x8000];
        sa1.io.bbf = true; // 2bpp
        sa1.bwram_write_bitmap(&mut bwram, 0, 0x01);
        sa1.bwram_write_bitmap(&mut bwram, 2, 0x03);
        assert_eq!(bwram[0], 0x31);
        assert_eq!(sa1.bwram_read_bitmap(&bwram, 0, 0), 0x01);
        assert_eq!(sa1.bwram_read_bitmap(&bwram, 2, 0), 0x03);
        let _ = rom;
    }

    #[test]
    fn bwram_write_protection() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];
        // Power-on: SWEN and CWEN clear, BWPA = $0f -> everything protected.
        sa1.write_cpu_bwram(0x00_6000, &mut bwram, 0x5A);
        assert_eq!(bwram[0], 0x00);
        // Enabling either write enable lifts the protection.
        sa1.write_cpu_io(0x2226, &rom, &mut bwram, 0x80);
        sa1.write_cpu_bwram(0x00_6000, &mut bwram, 0x5A);
        assert_eq!(bwram[0], 0x5A);
    }

    #[test]
    fn iram_write_protection() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        // S-CPU writes need SIWP bit 0 (ares maps the address first).
        sa1.write_cpu_iram(0x00_3000, 0x12);
        assert_eq!(sa1.iram[0], 0x00);
        sa1.write_cpu_io(0x2229, &rom, &mut [], 0x01);
        sa1.write_cpu_iram(0x00_3000, 0x12);
        assert_eq!(sa1.iram[0], 0x12);
        // SA-1 writes use the 256-byte block index.
        sa1.write_sa1(0x00_3100, &rom, &mut [], 0x34);
        assert_eq!(sa1.iram[0x100], 0x00);
        sa1.write_io_sa1(0x222A, &rom, &mut [], 0x02);
        sa1.write_sa1(0x00_3100, &rom, &mut [], 0x34);
        assert_eq!(sa1.iram[0x100], 0x34);
    }

    #[test]
    fn arithmetic_unit() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];

        // Signed multiply: -2 * 3 = -6.
        sa1.write_io_sa1(0x2250, &rom, &mut bwram, 0x00);
        sa1.write_io_sa1(0x2251, &rom, &mut bwram, 0xFE);
        sa1.write_io_sa1(0x2252, &rom, &mut bwram, 0xFF);
        sa1.write_io_sa1(0x2253, &rom, &mut bwram, 0x03);
        sa1.write_io_sa1(0x2254, &rom, &mut bwram, 0x00);
        assert_eq!(sa1.io.mr, 0xFFFF_FFFA);
        assert_eq!(sa1.io.mb, 0);

        // Unsigned divide: 100 / 7 = 14 remainder 2.
        sa1.write_io_sa1(0x2250, &rom, &mut bwram, 0x01);
        sa1.write_io_sa1(0x2251, &rom, &mut bwram, 100);
        sa1.write_io_sa1(0x2252, &rom, &mut bwram, 0);
        sa1.write_io_sa1(0x2253, &rom, &mut bwram, 7);
        sa1.write_io_sa1(0x2254, &rom, &mut bwram, 0);
        assert_eq!(sa1.io.mr, (2 << 16) | 14);
        assert_eq!((sa1.io.ma, sa1.io.mb), (0, 0));
    }

    #[test]
    fn dma_normal_copies_rom_into_bwram() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];
        // Enable DMA, source ROM, destination BWRAM.
        sa1.write_io_sa1(0x2230, &rom, &mut bwram, 0x80 | 0x04);
        sa1.write_io_sa1(0x2232, &rom, &mut bwram, 0x00);
        sa1.write_io_sa1(0x2233, &rom, &mut bwram, 0x80);
        sa1.write_io_sa1(0x2234, &rom, &mut bwram, 0x01); // source $018000 -> ROM $8000
        sa1.write_io_sa1(0x2238, &rom, &mut bwram, 4);
        sa1.write_io_sa1(0x2239, &rom, &mut bwram, 0);
        sa1.write_io_sa1(0x2235, &rom, &mut bwram, 0x00);
        sa1.write_io_sa1(0x2236, &rom, &mut bwram, 0x00);
        sa1.write_io_sa1(0x2237, &rom, &mut bwram, 0x00); // triggers
        assert_eq!(bwram[0], 0x20);
        assert_eq!(sa1.io.dtc, 0xFFFF);
        assert!(sa1.io.dma_irqfl);
    }

    #[test]
    fn timers_trigger_a_sa1_irq() {
        let mut sa1 = Sa1::new(Region::Ntsc);
        sa1.io.hen = true;
        sa1.io.hcnt = 10; // in dots -> 40 clocks
        sa1.io.timer_irqen = true;
        for _ in 0..20 {
            sa1.timer_step();
        }
        assert!(sa1.io.timer_irqfl);
        assert!(sa1.hcounter >= 40);
    }

    #[test]
    fn s_cpu_only_sees_the_flag_register() {
        let rom = rom();
        let mut sa1 = Sa1::new(Region::Ntsc);
        let mut bwram = vec![0u8; 0x8000];
        sa1.io.cmeg = 0x0A;
        sa1.io.cpu_irqfl = true;
        assert_eq!(sa1.read_cpu_io(0x2300, &rom, &mut bwram, 0x5A), 0x8A);
        // Every other address returns open bus.
        assert_eq!(sa1.read_cpu_io(0x2301, &rom, &mut bwram, 0x5A), 0x5A);
        // Writes to SA-1-only registers from the S-CPU are ignored.
        sa1.write_cpu_io(0x2209, &rom, &mut bwram, 0xFF);
        assert_eq!(sa1.io.cmeg, 0x0A);
    }
}
