//! Minimal N64 machine: RDRAM, RSP memory, cart ROM and the register stubs
//! needed to run a cartridge's IPL3 boot code on the VR4300.
//!
//! This HLEs the PIF (IPL1/IPL2): the cartridge's 4 KiB IPL3 is copied from
//! ROM offset `0x40` into RSP DMEM and the CPU starts at `0xA4000040`, exactly
//! where the real boot ROM hands off. PI DMA is modelled instantly so IPL3 can
//! load the game's boot code into RDRAM. RSP/RDP rendering, audio, the TLB and
//! precise exceptions are later milestones, so frames are blank.

use crate::cpu::{Cpu, CpuError};
use crate::Cartridge;

/// Instruction budget for one presented frame (placeholder until VI timing is
/// modelled from the RCP).
pub const INSTRUCTIONS_PER_FRAME: u32 = 200_000;

/// One N64 machine.
pub struct Machine {
    /// Architectural CPU.
    pub cpu: Cpu,
    /// Work RAM (4 MiB).
    rdram: Vec<u8>,
    /// RSP data memory (4 KiB).
    dmem: Vec<u8>,
    /// RSP instruction memory (4 KiB).
    imem: Vec<u8>,
    /// Cartridge image (normalised big-endian).
    rom: Vec<u8>,
    /// Presented frames.
    pub frames: u64,
    vi_line: u32,
    vi_intr: u32,
    pi_dram: u32,
    pi_cart: u32,
}

impl Machine {
    /// Builds a machine and boots the cartridge's IPL3.
    #[must_use]
    pub fn new(cart: &Cartridge) -> Self {
        let rom = cart.rom().to_vec();
        let mut dmem = vec![0u8; 0x1000];
        // PIF HLE: load IPL3 (4 KiB at ROM 0x40) into DMEM and start at +0x40.
        let end = (0x40 + 0x1000).min(rom.len());
        if end > 0x40 {
            let length = (end - 0x40).min(dmem.len());
            dmem[..length].copy_from_slice(&rom[0x40..0x40 + length]);
        }
        Self {
            cpu: Cpu::new(0xA400_0040),
            rdram: vec![0; 4 * 1024 * 1024],
            dmem,
            imem: vec![0; 0x1000],
            rom,
            frames: 0,
            vi_line: 0,
            vi_intr: 0,
            pi_dram: 0,
            pi_cart: 0,
        }
    }

    /// Work RAM.
    #[must_use]
    pub fn rdram(&self) -> &[u8] {
        &self.rdram
    }

    /// Executes one instruction.
    pub fn step(&mut self) -> Result<(), CpuError> {
        let mut cpu = std::mem::replace(&mut self.cpu, Cpu::new(0));
        let result = cpu.step(self);
        self.cpu = cpu;
        result
    }

    /// Runs one placeholder frame and returns its index.
    pub fn run_frame(&mut self) -> Result<u64, CpuError> {
        for _ in 0..INSTRUCTIONS_PER_FRAME {
            self.step()?;
        }
        self.frames += 1;
        self.vi_line = (self.vi_line + 1) % 525;
        Ok(self.frames)
    }
}

impl crate::cpu::Bus for Machine {
    fn read_u8(&mut self, address: u64) -> u8 {
        let address = address as u32;
        match address {
            0x0000_0000..=0x003F_FFFF => self.rdram[address as usize],
            0x0400_0000..=0x0400_0FFF => self.dmem[(address & 0xFFF) as usize],
            0x0400_1000..=0x0400_1FFF => self.imem[(address & 0xFFF) as usize],
            0x1000_0000..=0x1FBF_FFFF => self
                .rom
                .get((address - 0x1000_0000) as usize)
                .copied()
                .unwrap_or(0),
            _ => (self.read_register(address) >> ((3 - (address & 3)) * 8)) as u8,
        }
    }

    fn write_u8(&mut self, address: u64, value: u8) {
        let address = address as u32;
        match address {
            0x0000_0000..=0x003F_FFFF => self.rdram[address as usize] = value,
            0x0400_0000..=0x0400_0FFF => self.dmem[(address & 0xFFF) as usize] = value,
            0x0400_1000..=0x0400_1FFF => self.imem[(address & 0xFFF) as usize] = value,
            _ => {}
        }
    }

    fn read_u32(&mut self, address: u64) -> u32 {
        let address = address as u32;
        match address {
            0x0000_0000..=0x003F_FFFF => {
                let mut bytes = [0u8; 4];
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self.rdram[address as usize + offset];
                }
                u32::from_be_bytes(bytes)
            }
            0x0400_0000..=0x0400_0FFF => {
                let mut bytes = [0u8; 4];
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self.dmem[(address & 0xFFF) as usize + offset];
                }
                u32::from_be_bytes(bytes)
            }
            0x0400_1000..=0x0400_1FFF => {
                let mut bytes = [0u8; 4];
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self.imem[(address & 0xFFF) as usize + offset];
                }
                u32::from_be_bytes(bytes)
            }
            0x1000_0000..=0x1FBF_FFFF => {
                let mut bytes = [0u8; 4];
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self
                        .rom
                        .get((address - 0x1000_0000) as usize + offset)
                        .copied()
                        .unwrap_or(0);
                }
                u32::from_be_bytes(bytes)
            }
            _ => self.read_register(address),
        }
    }

    fn write_u32(&mut self, address: u64, value: u32) {
        let address = address as u32;
        match address {
            0x0000_0000..=0x003F_FFFF => {
                self.rdram[address as usize..address as usize + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            0x0400_0000..=0x0400_0FFF => {
                self.dmem[(address & 0xFFF) as usize..(address & 0xFFF) as usize + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            0x0400_1000..=0x0400_1FFF => {
                self.imem[(address & 0xFFF) as usize..(address & 0xFFF) as usize + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            0x0460_0000 => self.pi_dram = value,
            0x0460_0004 => self.pi_cart = value,
            0x0460_0008 => self.pi_dma(value),
            0x0460_000C => self.pi_dma(value),
            0x0440_000C => self.vi_intr = value,
            _ => {}
        }
    }
}

impl Machine {
    fn pi_dma(&mut self, length: u32) {
        let length = length.min(0x0080_0000);
        let source = self.pi_cart as usize;
        let destination = self.pi_dram as usize;
        for offset in 0..length as usize {
            let byte = self.rom.get(source + offset).copied().unwrap_or(0);
            if let Some(slot) = self.rdram.get_mut(destination + offset) {
                *slot = byte;
            }
        }
    }

    fn read_register(&mut self, address: u32) -> u32 {
        match address {
            0x0440_0010 => self.vi_line,
            0x0440_000C => self.vi_intr,
            0x0460_0000 => self.pi_dram,
            0x0460_0004 => self.pi_cart,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ByteOrder;

    fn cart_with(program: &[u32]) -> Cartridge {
        let mut rom = vec![0u8; 0x200];
        rom[0x00..0x04].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        rom[0x20..0x20 + 8].copy_from_slice(b"TEST    ");
        rom[0x3C] = b'N';
        rom[0x3D] = b'D';
        rom[0x3E] = b'E';
        for (index, word) in program.iter().enumerate() {
            let offset = 0x80 + index * 4;
            rom[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
        }
        Cartridge::from_bytes(&rom).expect("cart")
    }

    #[test]
    fn boots_ipl3_from_dmem() {
        // lui t0,0x8000 ; addiu t1,zero,0x1234 ; sw t1,0(t0) ; beq zero,zero,-1
        let cart = cart_with(&[0x3C08_8000, 0x2409_1234, 0xAD09_0000, 0x1000_FFFF]);
        let mut machine = Machine::new(&cart);
        assert_eq!(machine.cpu.pc, 0xA400_0040);
        for _ in 0..3 {
            machine.step().expect("step");
        }
        assert_eq!(&machine.rdram()[..4], &[0x00, 0x00, 0x12, 0x34]);
        assert_eq!(machine.run_frame().expect("frame"), 1);
    }

    #[test]
    fn pi_dma_copies_cart_into_rdram() {
        let cart = cart_with(&[]);
        let mut machine = Machine::new(&cart);
        machine.pi_cart = 0x0000_0100;
        machine.pi_dram = 0x0000_2000;
        machine.pi_dma(16);
        assert_eq!(&machine.rdram()[0x2000..0x2004], &[0, 0, 0, 0]);
    }

    #[test]
    fn byte_order_is_big_endian() {
        let cart = cart_with(&[]);
        assert_eq!(cart.byte_order(), ByteOrder::Big);
    }
}
