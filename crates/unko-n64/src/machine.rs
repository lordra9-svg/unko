//! Minimal N64 machine: RDRAM, the RSP/RDP scaffolds, cart ROM and the register
//! stubs needed to boot a cartridge's IPL3 and present a framebuffer.
//!
//! The PIF is HLE'd: the cartridge's 4 KiB IPL3 is copied from ROM offset
//! `0x40` into RSP DMEM and the CPU starts at `0xA4000040`, the real handoff
//! point. PI DMA is modelled instantly so IPL3 can load the game's boot code.
//! `run_frame` boots a placeholder frame, runs a bounded amount of RSP scalar
//! code, applies pending RDP fill commands and presents the VI framebuffer from
//! RDRAM as RGBA. Textured triangles, audio, the TLB and FPU are later.

use crate::cpu::{Bus, Cpu, CpuError};
use crate::rdp::Rdp;
use crate::rsp::Rsp;
use crate::Cartridge;

/// CPU instruction budget for one presented frame.
pub const INSTRUCTIONS_PER_FRAME: u32 = 200_000;
/// RSP scalar instruction budget for one presented frame.
pub const RSP_INSTRUCTIONS_PER_FRAME: u32 = 4_096;
/// Presented width.
pub const WIDTH: u32 = 640;
/// Presented height.
pub const HEIGHT: u32 = 480;

/// One N64 machine.
pub struct Machine {
    /// Architectural CPU.
    pub cpu: Cpu,
    /// RSP scaffold.
    pub rsp: Rsp,
    /// RDP scaffold.
    pub rdp: Rdp,
    /// Work RAM (4 MiB).
    rdram: Vec<u8>,
    /// Cartridge image (normalised big-endian).
    rom: Vec<u8>,
    /// Presented frames.
    pub frames: u64,
    frame: Vec<u8>,
    vi_origin: u32,
    vi_width: u32,
    vi_line: u32,
    pi_dram: u32,
    pi_cart: u32,
}

impl Machine {
    /// Builds a machine and boots the cartridge's IPL3.
    #[must_use]
    pub fn new(cart: &Cartridge) -> Self {
        let rom = cart.rom().to_vec();
        let mut rsp = Rsp::new();
        // PIF HLE: load IPL3 (4 KiB at ROM 0x40) into DMEM and start at +0x40.
        let end = (0x40 + 0x1000).min(rom.len());
        if end > 0x40 {
            let length = (end - 0x40).min(rsp.dmem.len());
            rsp.dmem[..length].copy_from_slice(&rom[0x40..0x40 + length]);
        }
        Self {
            cpu: Cpu::new(0xA400_0040),
            rsp,
            rdp: Rdp::new(),
            rdram: vec![0; 4 * 1024 * 1024],
            rom,
            frames: 0,
            frame: vec![0; (WIDTH * HEIGHT * 4) as usize],
            vi_origin: 0,
            vi_width: 320,
            vi_line: 0,
            pi_dram: 0,
            pi_cart: 0,
        }
    }

    /// Work RAM.
    #[must_use]
    pub fn rdram(&self) -> &[u8] {
        &self.rdram
    }

    /// Mutable work RAM (tests/diagnostics).
    pub fn rdram_mut(&mut self) -> &mut [u8] {
        &mut self.rdram
    }

    /// Most recently presented RGBA frame.
    #[must_use]
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// Executes one CPU instruction.
    pub fn step(&mut self) -> Result<(), CpuError> {
        let mut cpu = std::mem::replace(&mut self.cpu, Cpu::new(0));
        let result = cpu.step(self);
        self.cpu = cpu;
        result
    }

    /// Runs one placeholder frame and returns its index.
    pub fn run_frame(&mut self) -> Result<u64, CpuError> {
        if !self.rsp.halted {
            self.rsp.run(RSP_INSTRUCTIONS_PER_FRAME);
        }
        for _ in 0..INSTRUCTIONS_PER_FRAME {
            self.step()?;
        }
        self.frames += 1;
        self.vi_line = (self.vi_line + 1) % 525;
        self.present();
        Ok(self.frames)
    }

    fn present(&mut self) {
        self.frame.fill(0);
        let width = self.vi_width.min(WIDTH) as usize;
        if width == 0 {
            return;
        }
        let height = 240usize;
        for y in 0..height {
            for x in 0..width {
                let index = self.vi_origin as usize + (y * width + x) * 2;
                let Some(bytes) = self.rdram.get(index..index + 2) else {
                    continue;
                };
                let color = u16::from_be_bytes([bytes[0], bytes[1]]);
                let r = expand5((color >> 11) & 0x1F);
                let g = expand5((color >> 6) & 0x1F);
                let b = expand5((color >> 1) & 0x1F);
                let a = if color & 1 != 0 { 0xFF } else { 0x00 };
                let out = (y * WIDTH as usize + x) * 4;
                self.frame[out..out + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
    }
}

fn expand5(value: u16) -> u8 {
    let value = value as u8;
    (value << 3) | (value >> 2)
}

impl Bus for Machine {
    fn read_u8(&mut self, address: u64) -> u8 {
        let address = address as u32;
        match address {
            0x0000_0000..=0x003F_FFFF => self.rdram[address as usize],
            0x0400_0000..=0x0400_0FFF => self.rsp.dmem[(address & 0xFFF) as usize],
            0x0400_1000..=0x0400_1FFF => self.rsp.imem[(address & 0xFFF) as usize],
            0x0404_0000..=0x0404_003F => {
                (self.rsp.read_register(address - 0x0404_0000) >> ((3 - (address & 3)) * 8)) as u8
            }
            0x0410_0000..=0x0410_001F => {
                (self.rdp.read_register(address - 0x0410_0000) >> ((3 - (address & 3)) * 8)) as u8
            }
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
            0x0400_0000..=0x0400_0FFF => self.rsp.dmem[(address & 0xFFF) as usize] = value,
            0x0400_1000..=0x0400_1FFF => self.rsp.imem[(address & 0xFFF) as usize] = value,
            _ => {}
        }
    }

    fn read_u32(&mut self, address: u64) -> u32 {
        let address = address as u32;
        match address {
            0x0404_0000..=0x0404_003C => self.rsp.read_register(address - 0x0404_0000),
            0x0410_0000..=0x0410_001C => self.rdp.read_register(address - 0x0410_0000),
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
                    *byte = self.rsp.dmem[(address & 0xFFF) as usize + offset];
                }
                u32::from_be_bytes(bytes)
            }
            0x0400_1000..=0x0400_1FFF => {
                let mut bytes = [0u8; 4];
                for (offset, byte) in bytes.iter_mut().enumerate() {
                    *byte = self.rsp.imem[(address & 0xFFF) as usize + offset];
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
                self.rsp.dmem[(address & 0xFFF) as usize..(address & 0xFFF) as usize + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            0x0400_1000..=0x0400_1FFF => {
                self.rsp.imem[(address & 0xFFF) as usize..(address & 0xFFF) as usize + 4]
                    .copy_from_slice(&value.to_be_bytes());
            }
            0x0404_0000..=0x0404_003C => {
                self.rsp
                    .write_register(address - 0x0404_0000, value, &mut self.rdram);
            }
            0x0410_0000..=0x0410_001C => {
                self.rdp
                    .write_register(address - 0x0410_0000, value, &mut self.rdram);
            }
            _ => self.write_register(address, value),
        }
    }
}

impl Machine {
    fn read_register(&mut self, address: u32) -> u32 {
        match address {
            0x0440_0000 => 0, // VI status
            0x0440_0004 => self.vi_origin,
            0x0440_0008 => self.vi_width,
            0x0440_000C => 0,
            0x0440_0010 => self.vi_line,
            0x0460_0000 => self.pi_dram,
            0x0460_0004 => self.pi_cart,
            _ => 0,
        }
    }

    fn write_register(&mut self, address: u32, value: u32) {
        match address {
            0x0440_0004 => self.vi_origin = value & 0x00FF_FFFF,
            0x0440_0008 => self.vi_width = value & 0x0FFF,
            0x0460_0000 => self.pi_dram = value,
            0x0460_0004 => self.pi_cart = value,
            0x0460_0008 | 0x0460_000C => self.pi_dma(value),
            _ => {}
        }
    }

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
    fn rdp_fill_is_presented_from_rdram() {
        // A forever loop keeps the CPU in valid code while the RDP renders.
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x1000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 64);
        let commands = [
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (64u64 << 32) | u64::from(origin),
            (0x37u64 << 56) | 0xFF00_00FF,
            (0x36u64 << 56) | (3u64 << 12) | 3,
        ];
        for (index, command) in commands.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (command >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *command as u32);
        }
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, 24);
        machine.run_frame().expect("frame");
        assert_eq!(&machine.frame()[..4], &[0xFF, 0x00, 0x00, 0xFF]);
        let outside = 4 * 4;
        assert_eq!(&machine.frame()[outside..outside + 4], &[0, 0, 0, 0]);
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
