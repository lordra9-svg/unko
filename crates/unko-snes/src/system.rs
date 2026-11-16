//! Minimal SNES system bus and CPU frame driver.
//!
//! The PPU, SPC700/S-DSP and DMA engine are not implemented yet. This module
//! currently provides the S-CPU memory map, WRAM, cartridge access and a bus
//! implementation suitable for CPU tests and for booting retail images far
//! enough to exercise the 65C816.

use crate::apu::Apu;
use crate::cpu::{Cpu, CpuBus};
use crate::rom::Cartridge;

/// Size of the SNES work RAM.
pub const WRAM_SIZE: usize = 0x2_0000;

/// A SNES system with a CPU and cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snes {
    /// The S-CPU.
    pub cpu: Cpu,
    /// The inserted cartridge.
    pub cart: Cartridge,
    /// The S-SMP/S-DSP audio unit.
    pub apu: Apu,
    wram: Vec<u8>,
    apu_credit: i64,
}

impl Snes {
    /// Create a system around a parsed cartridge.
    pub fn new(cart: Cartridge) -> Self {
        Self {
            cpu: Cpu::new(),
            cart,
            apu: Apu::new(),
            wram: vec![0; WRAM_SIZE],
            apu_credit: 0,
        }
    }

    /// Work RAM.
    pub fn wram(&self) -> &[u8] {
        &self.wram
    }

    /// Mutable work RAM, for save states and tests.
    pub fn wram_mut(&mut self) -> &mut [u8] {
        &mut self.wram
    }

    /// Install a user-supplied 64-byte SPC700 IPL ROM.
    pub fn set_ipl(&mut self, ipl: [u8; crate::apu::IPL_SIZE]) {
        self.apu.set_ipl(ipl);
    }

    /// Reset the CPU and fetch the reset vector through the system bus.
    pub fn reset(&mut self) -> u32 {
        self.apu.reset();
        self.apu_credit = 0;
        let mut bus = SnesBus {
            wram: &mut self.wram,
            cart: &mut self.cart,
            apu: &mut self.apu,
        };
        self.cpu.reset(&mut bus)
    }

    /// Execute one CPU instruction or interrupt.
    pub fn step(&mut self) -> u32 {
        let cycles = {
            let mut bus = SnesBus {
                wram: &mut self.wram,
                cart: &mut self.cart,
                apu: &mut self.apu,
            };
            self.cpu.step(&mut bus)
        };
        // NTSC S-CPU bus cycles are 3.579545 MHz and the SPC700 runs at
        // 1.024 MHz. The credit is carried across instructions so long-term
        // APU timing stays in step.
        self.apu_credit += i64::from(cycles) * 1_024_000;
        while self.apu_credit >= 3_579_545 {
            let spent = self.apu.step();
            self.apu_credit -= i64::from(spent) * 3_579_545;
        }
        cycles
    }

    /// Run until `max_cycles` bus cycles have elapsed, the CPU stops, or an
    /// instruction limit is reached.
    pub fn run_cycles(&mut self, max_cycles: u64, max_instructions: u32) -> (u64, u32) {
        let mut cycles = 0;
        let mut instructions = 0;
        while cycles < max_cycles && instructions < max_instructions && !self.cpu.stp {
            cycles += u64::from(self.step());
            instructions += 1;
        }
        (cycles, instructions)
    }
}

struct SnesBus<'a> {
    wram: &'a mut [u8],
    cart: &'a mut Cartridge,
    apu: &'a mut Apu,
}

impl CpuBus for SnesBus<'_> {
    fn read(&mut self, address: u32) -> u8 {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            return self.wram[((bank as usize & 1) << 16) | offset];
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                return self.wram[offset & 0x1FFF];
            }
            if offset < 0x6000 {
                if (0x2140..=0x2143).contains(&offset) {
                    return self.apu.cpu_read(offset - 0x2140);
                }
                return 0;
            }
            if offset < 0x8000 {
                return 0;
            }
        }
        self.cart.read(address).unwrap_or(0)
    }

    fn write(&mut self, address: u32, value: u8) {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            self.wram[((bank as usize & 1) << 16) | offset] = value;
            return;
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                self.wram[offset & 0x1FFF] = value;
                return;
            }
            if offset < 0x6000 {
                if (0x2140..=0x2143).contains(&offset) {
                    self.apu.cpu_write(offset - 0x2140, value);
                }
                return;
            }
            if offset < 0x8000 {
                return;
            }
        }
        self.cart.write(address, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::{Mapping, RomHeader};

    fn build_lorom(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x1_0000];
        let header = Mapping::LoRom.header_offset();
        rom[header..header + 8].copy_from_slice(b"TEST ROM");
        rom[header + 0x15] = 0x20;
        rom[header + 0x16] = 0x02;
        rom[header + 0x17] = 8;
        rom[header + 0x18] = 3;
        rom[header + 0x19] = 1;
        rom[header + 0x1A] = 1;
        rom[header + 0x3C..header + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
        rom[0..code.len()].copy_from_slice(code);

        let mut checksum_source = rom.clone();
        checksum_source[header + 0x1C..header + 0x20].fill(0);
        let checksum = (checksum_source
            .iter()
            .map(|&byte| u32::from(byte))
            .sum::<u32>()
            & 0xFFFF) as u16;
        rom[header + 0x1C..header + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        rom[header + 0x1E..header + 0x20].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    fn cartridge(code: &[u8]) -> Cartridge {
        Cartridge::from_bytes(&build_lorom(code)).expect("valid LoROM")
    }

    #[test]
    fn decodes_header_and_maps_wram() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        assert_eq!(system.cart.header().mapping, Mapping::LoRom);
        assert_eq!(system.cart.header().title, "TEST ROM");
        assert_eq!(system.cart.header().reset_vector, 0x8000);
        system.wram_mut()[0x1234] = 0x5A;
        assert_eq!(system.wram()[0x1234], 0x5A);

        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            apu: &mut system.apu,
        };
        assert_eq!(bus.read(0x001234), 0x5A);
        assert_eq!(bus.read(0x801234), 0x5A);
        assert_eq!(bus.read(0x7E1234), 0x5A);
        assert_eq!(bus.read(0x7F1234), 0x00);
    }

    #[test]
    fn reset_vector_and_execution_from_rom() {
        // LDA #$42; STP
        let mut system = Snes::new(cartridge(&[0xA9, 0x42, 0xDB]));
        system.reset();
        assert_eq!(system.cpu.pc, 0x8000);
        assert!(system.cpu.emulation);
        let (cycles, instructions) = system.run_cycles(100, 100);
        assert!(system.cpu.stp, "CPU did not reach STP");
        assert_eq!(system.cpu.a, 0x42);
        assert_eq!(instructions, 2);
        assert!(cycles >= 5);
    }

    #[test]
    fn maps_cartridge_reads_and_sram() {
        let mut system = Snes::new(cartridge(&[0x11, 0x22, 0x33]));
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
            };
            assert_eq!(bus.read(0x008000), 0x11);
            assert_eq!(bus.read(0x018000), 0x00);
            assert_eq!(bus.read(0x808000), 0x11);
            assert!(bus.cart.write(0x700000, 0x5A));
            assert_eq!(bus.read(0x700000), 0x5A);
        }
    }

    #[test]
    fn cpu_writes_reach_the_apu_ports() {
        // LDA #$5A; STA $2140; STP
        let mut system = Snes::new(cartridge(&[0xA9, 0x5A, 0x8D, 0x40, 0x21, 0xDB]));
        system.reset();
        system.run_cycles(100, 100);
        assert!(system.cpu.stp);
        assert_eq!(system.apu.apu_port(0), 0x5A);
    }

    #[test]
    fn cpu_reads_apu_output_ports() {
        // LDA $2141; STP
        let mut system = Snes::new(cartridge(&[0xAD, 0x41, 0x21, 0xDB]));
        system.reset();
        system.apu.set_cpu_port(1, 0x34);
        system.run_cycles(100, 100);
        assert!(system.cpu.stp);
        assert_eq!(system.cpu.a, 0x34);
    }

    #[test]
    fn header_type_is_exposed() {
        let cartridge = cartridge(&[0xDB]);
        let header: &RomHeader = cartridge.header();
        assert_eq!(header.declared_sram_size, Some(0x2000));
        assert!(header.has_battery());
    }
}
