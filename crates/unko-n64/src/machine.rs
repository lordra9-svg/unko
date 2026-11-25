//! Minimal N64 machine: RDRAM, the RSP/RDP scaffolds, cart ROM and the register
//! stubs needed to boot a cartridge's IPL3 and present a framebuffer.
//!
//! The PIF is HLE'd: the cartridge's 4 KiB IPL3 is copied from ROM offset
//! `0x40` into RSP DMEM and the CPU starts at `0xA4000040`, the real handoff
//! point. PI DMA is modelled instantly so IPL3 can load the game's boot code.
//! `run_frame` boots a placeholder frame, runs a bounded amount of RSP scalar
//! code, applies pending RDP fill commands and presents the VI framebuffer from
//! RDRAM as RGBA. Textured triangles, audio, the TLB and FPU are later.

use crate::ai::Ai;
use crate::cpu::{Bus, Cpu, CpuError, ErrorSource};
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
    /// Audio interface.
    pub ai: Ai,
    /// Work RAM (4 MiB).
    rdram: Vec<u8>,
    /// Cartridge image (normalised big-endian).
    rom: Vec<u8>,
    /// Presented frames.
    pub frames: u64,
    frame: Vec<u8>,
    audio: Vec<(i16, i16)>,
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
            ai: Ai::new(),
            rdram: vec![0; 4 * 1024 * 1024],
            rom,
            frames: 0,
            frame: vec![0; (WIDTH * HEIGHT * 4) as usize],
            audio: Vec::new(),
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

    /// Drains host-rate audio produced by AI DMA since the last call.
    pub fn take_audio(&mut self) -> Vec<(i16, i16)> {
        std::mem::take(&mut self.audio)
    }

    /// Host output audio sample rate.
    #[must_use]
    pub fn audio_sample_rate(&self) -> f32 {
        self.ai.output_rate()
    }

    /// Takes an asynchronous error-level exception (NMI, watchdog, cache
    /// error or reset) on the CPU.
    pub fn take_error(&mut self, source: ErrorSource) {
        self.cpu.take_error(source);
    }

    /// Raises the CPU external NMI line.
    pub fn signal_nmi(&mut self) {
        self.cpu.signal_nmi();
    }

    /// Raises the CPU external cache-error line.
    pub fn signal_cache_error(&mut self) {
        self.cpu.signal_cache_error();
    }

    /// Whether the AI DMA interrupt is pending.
    #[must_use]
    pub fn audio_interrupt_pending(&self) -> bool {
        self.ai.interrupt_pending()
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
        // Resample the queued AI stream to the host rate for one frame.
        let samples = (self.ai.output_rate() / 60.0).round() as usize;
        self.ai.generate(samples, &self.rdram, &mut self.audio);
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
            0x0450_0000..=0x0450_0017 => {
                (self.ai.read_register(address - 0x0450_0000) >> ((3 - (address & 3)) * 8)) as u8
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
            0x0450_0000..=0x0450_0014 => self.ai.read_register(address - 0x0450_0000),
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
            0x0450_0000..=0x0450_0014 => {
                self.ai
                    .write_register(address - 0x0450_0000, value, &self.rdram);
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

    fn pack_fixed(value: f32) -> (u64, u64) {
        let fixed = (value * 65_536.0).round() as i64;
        ((fixed >> 16) as u16 as u64, (fixed & 0xFFFF) as u64)
    }

    fn point_word(coordinate: f32, slope: f32) -> u64 {
        let (ci, cf) = pack_fixed(coordinate);
        let (si, sf) = pack_fixed(slope);
        (ci << 48) | (cf << 32) | (si << 16) | sf
    }

    /// Encode the four triangle edge words from three vertices sorted by Y.
    fn triangle_edge(opcode: u64, vertices: [(f32, f32); 3]) -> [u64; 4] {
        let [(x0, y0), (x1, y1), (x2, y2)] = vertices;
        let major = (x2 - x0) / (y2 - y0);
        let upper = (x1 - x0) / (y1 - y0);
        let lower = (x2 - x1) / (y2 - y1);
        let yh = (y0 * 4.0).round() as i64;
        let ym = (y1 * 4.0).round() as i64;
        let yl = (y2 * 4.0).round() as i64;
        let yh_base = (yh & !3) as f32 / 4.0;
        let lmajor = major < upper;
        let word = (opcode << 56)
            | if lmajor { 1u64 << 55 } else { 0 }
            | ((yl as u64 & 0x3FFF) << 32)
            | ((ym as u64 & 0x3FFF) << 16)
            | (yh as u64 & 0x3FFF);
        [
            word,
            point_word(x1, lower),
            point_word(x0 + major * (yh_base - y0), major),
            point_word(x0 + upper * (yh_base - y0), upper),
        ]
    }

    fn z_words(z: f32, dzdx: f32, dzdy: f32) -> [u64; 2] {
        let (zi, zf) = pack_fixed(z);
        let (xi, xf) = pack_fixed(dzdx);
        let (yi, yf) = pack_fixed(dzdy);
        [(zi << 48) | (zf << 32) | (xi << 16) | xf, (yi << 16) | yf]
    }

    fn frame_pixel(machine: &Machine, x: usize, y: usize) -> [u8; 4] {
        let index = (y * WIDTH as usize + x) * 4;
        let frame = machine.frame();
        [
            frame[index],
            frame[index + 1],
            frame[index + 2],
            frame[index + 3],
        ]
    }

    fn submit_rdp(machine: &mut Machine, words: &[u64]) {
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
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
    fn ai_dma_produces_audio_samples() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        for (index, (left, right)) in [(1i16, -2i16), (3, -4), (5, -6), (7, -8)]
            .iter()
            .enumerate()
        {
            let mut word = [0u8; 4];
            word[..2].copy_from_slice(&left.to_be_bytes());
            word[2..].copy_from_slice(&right.to_be_bytes());
            machine.write_u32(0x100 + index as u64 * 4, u32::from_be_bytes(word));
        }
        machine.write_u32(0x0450_0008, 1); // control enable
        machine.write_u32(0x0450_0000, 0x100); // dram address
        machine.write_u32(0x0450_0010, 1013); // ~48 kHz
        machine.write_u32(0x0450_0004, 16); // 4 stereo samples
        assert!(machine.audio_interrupt_pending());
        machine.run_frame().expect("frame");
        let audio = machine.take_audio();
        // One frame at the 48 kHz host rate.
        assert_eq!(audio.len(), 800);
        assert_eq!(audio[0], (1, -2));
        assert!((audio[3].0 - 7).abs() <= 1, "{:?}", audio[3]);
        assert!(machine.audio_sample_rate() > 40_000.0);
        // AI_STATUS acknowledges the DMA interrupt.
        machine.write_u32(0x0450_000C, 1);
        assert!(!machine.audio_interrupt_pending());
    }

    #[test]
    fn ai_buffer_boundary_interrupts_and_host_rate() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        for (index, (left, right)) in [(1i16, -1i16), (2, -2), (3, -3), (4, -4)]
            .iter()
            .enumerate()
        {
            let mut word = [0u8; 4];
            word[..2].copy_from_slice(&left.to_be_bytes());
            word[2..].copy_from_slice(&right.to_be_bytes());
            machine.write_u32(0x100 + index as u64 * 4, u32::from_be_bytes(word));
        }
        machine.write_u32(0x0450_0008, 1); // enable
        machine.write_u32(0x0450_0010, 1013); // ~48 kHz DAC
        machine.write_u32(0x0450_0000, 0x100);
        machine.write_u32(0x0450_0004, 8); // first two samples
                                           // Acknowledge the start interrupt, then queue the second buffer.
        machine.write_u32(0x0450_000C, 1);
        machine.write_u32(0x0450_0000, 0x108);
        machine.write_u32(0x0450_0004, 8);
        machine.run_frame().expect("frame");
        let audio = machine.take_audio();
        assert_eq!(audio.len(), 800);
        assert_eq!(audio[0], (1, -1));
        assert!(
            machine.audio_interrupt_pending(),
            "second buffer promotion raises AI IRQ during the frame"
        );
        machine.write_u32(0x0450_000C, 1);
        assert!(!machine.audio_interrupt_pending());
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_ci_tlut_and_perspective() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let texture = 0x3000u32;
        let palette = 0x3100u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 64);
        // CI4 texture: row 0 indices [1,2,3,0], rows below repeat.
        let rows = [[1u8, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2], [0, 1, 2, 3]];
        for (row, indices) in rows.iter().enumerate() {
            for pair in 0..2 {
                let byte = (indices[pair * 2] << 4) | indices[pair * 2 + 1];
                machine.write_u8(texture as u64 + (row * 8 + pair) as u64, byte);
            }
        }
        // Palette: 1 red, 2 green, 3 blue.
        for (index, color) in [(1u16, 0xF801u16), (2, 0x07C1), (3, 0x003F)] {
            machine.write_u16(palette as u64 + u64::from(index) * 2, color);
        }
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Du64 << 56) | (2u64 << 53) | (4u64 << 32) | u64::from(texture));
        words.push((0x35u64 << 56) | (2u64 << 53) | (1u64 << 41));
        words.push((0x32u64 << 56) | (3u64 << 12) | 3);
        // LOAD_TLUT: entries 0..15 from texture address + 0x20 lines.
        words.push((0x30u64 << 56) | (32u64 << 32) | (15u64 << 12));
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (64u64 << 32) | u64::from(origin),
        );
        words.extend(triangle_edge(0x0A, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]));
        // s.c = 4.0, w.c = 2.0, s/t steps 0.375 -> s = 4/2 = 2 (column 2).
        words.push((4u64 << 48) | (2u64 << 16));
        words.push(0);
        words.push(0);
        words.push(0x6000u64 << 48); // s.x.f
        words.push(0);
        words.push(0);
        words.push(0);
        words.push(0x6000u64 << 32); // t.y.f
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
        machine.run_frame().expect("frame");
        let pixel = |x: usize, y: usize| {
            let index = (y * WIDTH as usize + x) * 4;
            let frame = machine.frame();
            [
                frame[index],
                frame[index + 1],
                frame[index + 2],
                frame[index + 3],
            ]
        };
        // s = 2, t = 0 -> row 0 col 2 -> index 3 -> palette blue.
        assert_eq!(pixel(5, 5), [0, 0, 0xFF, 0xFF], "CI index 3 blue");
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_blender_alpha_and_copy_cycles() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let texture = 0x5000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        for row in 0..4 {
            for column in 0..4 {
                machine.write_u16(texture as u64 + (row * 8 + column * 2), 0xF801);
            }
        }
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        // Fill the framebuffer opaque blue (RGBA order).
        words.push((0x37u64 << 56) | 0x0000_FFFF);
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        words.push((0x3Du64 << 56) | (2u64 << 51) | (4u64 << 32) | u64::from(texture));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41));
        words.push((0x32u64 << 56) | (3u64 << 12) | 3);
        // Primitive alpha 0x80 feeds the combiner alpha.
        words.push((0x3Au64 << 56) | 0x0000_0080);
        // 1-cycle: RGB = texel * texelAlpha, alpha = primitive alpha.
        let combine = (1u64 << 52) | (8u64 << 28) | (8u64 << 47) | (7u64 << 15) | (3u64 << 9);
        words.push((0x3Cu64 << 56) | combine);
        // 1 cycle, 1a = pixel, 1b = pixel alpha, 2a = memory, 2b = 1 - alpha.
        // Image read enables the framebuffer as the blend source; dither off
        // (mode 3) keeps the blend result exact for the assert.
        words.push((0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 22) | (1u64 << 6));
        words.extend(triangle_edge(0x0A, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]));
        words.extend([0u64; 8]); // texture coefficients: s = t = 0.
                                 // Copy cycle: the next triangle bypasses the blender.
        words.push((0x2Fu64 << 56) | (0xCu64 << 36) | (2u64 << 52));
        words.extend(triangle_edge(
            0x0A,
            [(20.0, 4.0), (26.0, 6.0), (20.0, 12.0)],
        ));
        words.extend([0u64; 8]);
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
        machine.run_frame().expect("frame");

        let pixel = |x: usize, y: usize| {
            let index = (y * WIDTH as usize + x) * 4;
            let frame = machine.frame();
            [
                frame[index],
                frame[index + 1],
                frame[index + 2],
                frame[index + 3],
            ]
        };
        // 0x80 alpha over opaque blue: red 253*16/32 = 126, blue 255*16/32 = 127,
        // then 5-bit colour-image truncation expands to 123.
        assert_eq!(
            pixel(5, 9),
            [123, 0, 123, 0xFF],
            "alpha blend of red over blue"
        );
        assert_eq!(
            pixel(22, 9),
            [0xFF, 0, 0, 0xFF],
            "copy cycle writes the texel"
        );
        assert_eq!(pixel(28, 20), [0, 0, 0xFF, 0xFF], "background fill");
    }

    #[test]
    fn rdp_depth_test_rejects_farther_triangles() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let depth = 0x6000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        // Clear the Z buffer to the maximum (far) depth.
        for index in 0..32 * 32 {
            machine.write_u16(depth as u64 + index * 2, 0xFFFF);
        }
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x3Eu64 << 56) | u64::from(depth));
        // z compare + z update, opaque mode, dither off.
        let other = (0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 5) | (1u64 << 4);
        words.push(other);
        let vertices = [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)];
        // Far red triangle first (passes against the cleared buffer).
        words.push((0x3Au64 << 56) | 0xFF00_00FF);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(28_672.0, 0.0, 0.0));
        // Nearer green triangle passes and overwrites.
        words.push((0x3Au64 << 56) | 0x00FF_00FF);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(16_384.0, 0.0, 0.0));
        // Farther yellow triangle must be rejected.
        words.push((0x3Au64 << 56) | 0xFFFF_00FF);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(24_576.0, 0.0, 0.0));
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
        machine.run_frame().expect("frame");

        let pixel = |x: usize, y: usize| {
            let index = (y * WIDTH as usize + x) * 4;
            let frame = machine.frame();
            [
                frame[index],
                frame[index + 1],
                frame[index + 2],
                frame[index + 3],
            ]
        };
        assert_eq!(pixel(5, 9), [0, 0xFF, 0, 0xFF], "nearest triangle wins");
        // Z update stored z_compress(0x10000) = 0x400 in the upper 14 bits.
        let offset = depth as usize + (9 * 32 + 5) * 2;
        assert_eq!(&machine.rdram()[offset..offset + 2], &[0x10, 0x00]);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_alpha_compare_rejects_low_alpha() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x37u64 << 56) | 0x0000_FFFF); // blue
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        words.push((0x39u64 << 56) | 0x0000_0080); // blend alpha threshold 0x80
                                                   // alpha compare, dither off; image read for blending,
                                                   // 2a = memory colour.
        let other = (0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 6) | 1 | (1u64 << 22);
        words.push(other);
        words.push((0x3Au64 << 56) | 0xFF00_0040); // red alpha 0x40 < threshold
        words.extend(triangle_edge(0x08, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("reject low alpha");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [0, 0, 0xFF, 0xFF],
            "low alpha rejected"
        );

        // Alpha 0xC0 passes and blends red over blue.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0xFF00_00C0);
        words.extend(triangle_edge(
            0x08,
            [(16.0, 4.0), (22.0, 6.0), (16.0, 12.0)],
        ));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("accept high alpha");
        assert_eq!(frame_pixel(&machine, 17, 9), [189, 0, 57, 0xFF]);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_interpenetrating_depth_scales_coverage() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let depth = 0x6000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        for index in 0..32 * 32 {
            machine.write_u16(depth as u64 + index * 2, 0xFFFF);
        }
        let vertices = [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)];
        // 1-cycle primitive colour with full alpha; AA + z test/update +
        // image read, interpenetrating z-mode, clamp coverage, dither off.
        let combine = (8u64 << 52)
            | (8u64 << 28)
            | (16u64 << 47)
            | (3u64 << 15)
            | (7u64 << 44)
            | (7u64 << 12)
            | (7u64 << 41)
            | (6u64 << 9);
        let other = (0x2Fu64 << 56)
            | (0xCu64 << 36)
            | (1u64 << 10)
            | (1u64 << 6)
            | (1u64 << 5)
            | (1u64 << 4)
            | (1u64 << 3)
            | (1u64 << 22);

        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x3Eu64 << 56) | u64::from(depth));
        words.push((0x37u64 << 56) | 0x0000_FFFF); // blue
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        words.push(other);
        words.push((0x3Cu64 << 56) | combine);
        words.push((0x3Au64 << 56) | 0xFF00_00FF); // red surface at z=0x20000
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(32_768.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("first surface");

        // Green fragment 0x10 in front: interpenetrating coverage scaling.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0x00FF_00FF);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(32_764.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("interpenetrating fragment");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [0, 0xFF, 0, 0],
            "reduced coverage yields partial alpha"
        );

        // A farther-in-front fragment (outside the delta window) keeps full
        // coverage and opaque alpha.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0x00FF_00FF);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(16_384.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("opaque fragment");
        assert_eq!(frame_pixel(&machine, 5, 9), [0, 0xFF, 0, 0xFF]);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_mip_lod_blends_tiles_and_feeds_combiner() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let texture = 0x3000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        for row in 0..4 {
            for column in 0..4 {
                machine.write_u16(texture as u64 + (row * 8 + column * 2), 0xF801); // red
                machine.write_u16(
                    texture as u64 + 0x20 + (row * 8 + column * 2),
                    0x07C1, // green
                );
            }
        }
        let vertices = [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)];
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x3Du64 << 56) | (2u64 << 51) | (4u64 << 32) | u64::from(texture));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41) | (1u64 << 24) | (4u64 << 32));
        words.push((0x32u64 << 56) | (3u64 << 12) | 3);
        words.push((0x32u64 << 56) | (1u64 << 24) | (3u64 << 12) | 3);
        // 1-cycle, dither off, texture LOD enabled.
        words.push((0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 48));
        let mut edge = triangle_edge(0x0A, vertices);
        edge[0] |= 1u64 << 51; // max LOD level 1
        words.extend(edge);
        words.push(0); // s/t/w c.i
        words.push(1u64 << 48); // s.x.i = 1 texel/pixel
        words.push(0); // s/t/w c.f
        words.push(0x6A00u64 << 48); // s.x.f = sqrt(2) - 1
        words.extend([0u64; 4]);
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("mip blend");
        // LOD = 0.5 -> half red (tile 0), half green (tile 1).
        assert_eq!(frame_pixel(&machine, 5, 9), [148, 107, 0, 0xFF]);

        // The same LOD fraction feeds the combiner LOD selector.
        let mut words: Vec<u64> = Vec::new();
        let combine = (6u64 << 52) | (8u64 << 28) | (13u64 << 47) | (7u64 << 15) | (6u64 << 9);
        words.push((0x3Cu64 << 56) | combine);
        let mut edge = triangle_edge(0x0A, [(20.0, 4.0), (26.0, 6.0), (20.0, 12.0)]);
        edge[0] |= 1u64 << 51;
        words.extend(edge);
        words.push(0);
        words.push(1u64 << 48);
        words.push(0);
        words.push(0x6A00u64 << 48);
        words.extend([0u64; 4]);
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("lod combiner");
        // LOD fraction 104 -> 0xFF * 104 / 256 = 103 -> 107 after expansion.
        assert_eq!(frame_pixel(&machine, 21, 9), [107, 107, 107, 0xFF]);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_perspective_lod_selects_tiles() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let texture = 0x3000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        for row in 0..4 {
            for column in 0..4 {
                machine.write_u16(texture as u64 + (row * 8 + column * 2), 0xF801); // red
                machine.write_u16(texture as u64 + 0x20 + (row * 8 + column * 2), 0x07C1);
                // green
            }
        }
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x3Du64 << 56) | (2u64 << 51) | (4u64 << 32) | u64::from(texture));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41) | (1u64 << 24) | (4u64 << 32));
        words.push((0x32u64 << 56) | (3u64 << 12) | 3);
        words.push((0x32u64 << 56) | (1u64 << 24) | (3u64 << 12) | 3);
        words.push((0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 48)); // tex LOD
                                                                     // Constant W=1: linear derivative 2 texels/pixel selects mip level 1.
        let mut edge = triangle_edge(0x0A, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]);
        edge[0] |= 2u64 << 51; // max LOD level 2
        words.extend(edge);
        words.push(1u64 << 16); // w.c.i = 1 (constant W)
        words.push(2u64 << 48); // s.x.i = 2
        words.push(0);
        words.push(0);
        words.extend([0u64; 4]);
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("linear LOD");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [0, 0xFF, 0, 0xFF],
            "level 1 tile"
        );

        // Perspective W=1+x: s/w derivative is ~0.33 texel/pixel, magnifying
        // to level 0 (the red base tile).
        let mut words: Vec<u64> = Vec::new();
        let mut edge = triangle_edge(0x0A, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]);
        edge[0] |= 2u64 << 51;
        words.extend(edge);
        words.push(1u64 << 16); // w.c.i = 1
        words.push((2u64 << 48) | (1u64 << 16)); // s.x.i = 2, w.x.i = 1
        words.push(0);
        words.push(0);
        words.extend([0u64; 4]);
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("perspective LOD");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [0xFF, 0, 0, 0xFF],
            "level 0 tile"
        );
        assert_eq!(machine.rdp.tile_mip_level(1), 1);
        assert_eq!(machine.rdp.tile_mip_level(2), 2);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_combiner_k4_k5_lod_and_noise_inputs() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        let vertices_a = [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)];
        let vertices_b = [(14.0, 4.0), (20.0, 6.0), (14.0, 12.0)];
        let vertices_c = [(24.0, 4.0), (30.0, 6.0), (24.0, 12.0)];
        let alpha_one = 6u64 << 9;
        let white = (0x3Au64 << 56) | 0xFFFF_FFFF;

        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x37u64 << 56) | 0x0000_00FF); // black
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        words.push((0x2Fu64 << 56) | (0xCu64 << 36)); // 1-cycle, dither off
                                                      // Convert K5 = 0x80 and use it as the multiplier.
        words.push((0x2Cu64 << 56) | 0x80u64);
        let combine = (6u64 << 52) | (8u64 << 28) | (15u64 << 47) | (7u64 << 15) | alpha_one;
        words.push((0x3Cu64 << 56) | combine);
        words.push(white);
        words.extend(triangle_edge(0x08, vertices_a));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("K5 combiner");
        assert_eq!(frame_pixel(&machine, 5, 9), [132, 132, 132, 0xFF]);

        // Convert K4 = 0x40 subtracted, multiplied by PrimLODFrac = 0xFF.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x2Cu64 << 56) | (0x40u64 << 9));
        words.push((0x3Au64 << 56) | (0xFFu64 << 32) | 0xFFFF_FFFF);
        let combine = (6u64 << 52) | (7u64 << 28) | (14u64 << 47) | (7u64 << 15) | alpha_one;
        words.push((0x3Cu64 << 56) | combine);
        words.extend(triangle_edge(0x08, vertices_b));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("K4 combiner");
        assert_eq!(frame_pixel(&machine, 15, 9), [189, 189, 189, 0xFF]);

        // PrimLODFrac = 0x40 as the multiplier.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | (0x40u64 << 32) | 0xFFFF_FFFF);
        let combine = (6u64 << 52) | (8u64 << 28) | (14u64 << 47) | (7u64 << 15) | alpha_one;
        words.push((0x3Cu64 << 56) | combine);
        words.extend(triangle_edge(0x08, vertices_c));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("PrimLOD combiner");
        assert_eq!(frame_pixel(&machine, 25, 9), [66, 66, 66, 0xFF]);

        // Noise source: adjacent pixels differ.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | (0xFFu64 << 32) | 0xFFFF_FFFF);
        let combine = (7u64 << 52) | (8u64 << 28) | (14u64 << 47) | (7u64 << 15) | alpha_one;
        words.push((0x3Cu64 << 56) | combine);
        words.extend(triangle_edge(0x08, vertices_a));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("noise combiner");
        assert_ne!(frame_pixel(&machine, 5, 9), frame_pixel(&machine, 6, 9));
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_chroma_key_masks_matching_pixels() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x37u64 << 56) | 0x0000_FFFF); // blue
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        // key_enable (other modes bit 40), image read, 2a = memory colour.
        let other = (0x2Fu64 << 56) | (0xCu64 << 36) | (1u64 << 40) | (1u64 << 6) | (1u64 << 22);
        words.push(other);
        // Key windows: width 0x20 per channel, centre G=0xFF, scale 0x80.
        words.push(
            (0x2Au64 << 56)
                | (0x20u64 << 44)
                | (0x20u64 << 32)
                | (0xFFu64 << 24)
                | (0x80u64 << 16)
                | 0x80,
        );
        words.push((0x2Bu64 << 56) | (0x20u64 << 16) | 0x80);
        // 1-cycle: A=Primitive, B=KeyCentre, C=KeyScale, D=0; alpha=primitive.
        let combine = (3u64 << 52)
            | (6u64 << 28)
            | (6u64 << 47)
            | (7u64 << 15)
            | (7u64 << 44)
            | (7u64 << 12)
            | (7u64 << 41)
            | (3u64 << 9);
        words.push((0x3Cu64 << 56) | combine);
        words.push((0x3Au64 << 56) | 0xFF00_00FF);
        words.extend(triangle_edge(0x08, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("keyed red");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [0, 0, 0xFF, 0],
            "red keyed out"
        );

        // Green sits at its key centre (G=0xFF), so it is kept.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0x00FF_00FF);
        words.extend(triangle_edge(
            0x08,
            [(20.0, 4.0), (26.0, 6.0), (20.0, 12.0)],
        ));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("green kept");
        assert_eq!(frame_pixel(&machine, 21, 9), [0, 0xFF, 0, 0xFF]);
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_multilayer_aa_depth_blend() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let depth = 0x6000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 32);
        for index in 0..32 * 32 {
            machine.write_u16(depth as u64 + index * 2, 0xFFFF);
        }
        let vertices = [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)];
        let combine = (8u64 << 52)
            | (8u64 << 28)
            | (16u64 << 47)
            | (3u64 << 15)
            | (7u64 << 44)
            | (7u64 << 12)
            | (7u64 << 41)
            | (3u64 << 9);

        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (32u64 << 32) | u64::from(origin),
        );
        words.push((0x3Eu64 << 56) | u64::from(depth));
        words.push((0x37u64 << 56) | 0x0000_FFFF); // opaque blue
        words.push((0x36u64 << 56) | (31u64 << 12) | 31);
        // AA + z compare/update + image read + force blend, dither off,
        // 2a = memory colour.
        let other = (0x2Fu64 << 56)
            | (0xCu64 << 36)
            | (1u64 << 14)
            | (1u64 << 6)
            | (1u64 << 5)
            | (1u64 << 4)
            | (1u64 << 3)
            | (1u64 << 22);
        words.push(other);
        words.push((0x3Au64 << 56) | 0xFF00_0080); // red, alpha 0x80
        words.push((0x3Cu64 << 56) | combine);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(24_576.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("first layer");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [123, 0, 123, 0xFF],
            "red AA layer over blue"
        );

        // Second, nearer green layer blends over the first.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0x00FF_0080);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(16_384.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("second layer");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [57, 123, 57, 0xFF],
            "green AA layer over red"
        );

        // A farther red layer is rejected by the depth test.
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Au64 << 56) | 0xFF00_0080);
        words.extend(triangle_edge(0x09, vertices));
        words.extend(z_words(20_480.0, 0.0, 0.0));
        submit_rdp(&mut machine, &words);
        machine.run_frame().expect("third layer");
        assert_eq!(
            frame_pixel(&machine, 5, 9),
            [57, 123, 57, 0xFF],
            "farther layer rejected"
        );
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_textured_triangle_samples_the_tile() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        let texture = 0x3000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 64);
        // 4x4 RGBA5551 texture: red, green, blue, white rows.
        let rows = [0xF801u16, 0x07C1, 0x003F, 0xFFFF];
        for (row, color) in rows.iter().enumerate() {
            for column in 0..4 {
                machine.write_u16(texture as u64 + (row * 8 + column * 2) as u64, *color);
            }
        }
        let mut words: Vec<u64> = Vec::new();
        words.push((0x3Du64 << 56) | (2u64 << 51) | (4u64 << 32) | u64::from(texture));
        words.push((0x35u64 << 56) | (2u64 << 51) | (1u64 << 41));
        words.push((0x32u64 << 56) | (3u64 << 12) | 3);
        // Combiner: cycle0 = texture, cycle1 = pass-through of cycle0.
        // 1-cycle texel passthrough: A=Texel0, B=0, C=Texel0Alpha, D=0;
        // alpha A=One, B=0, C=Texel0Alpha, D=0.
        let combine = (1u64 << 52)
            | (8u64 << 28)
            | (8u64 << 47)
            | (7u64 << 15)
            | (6u64 << 44)
            | (7u64 << 12)
            | (1u64 << 41)
            | (7u64 << 9);
        words.push((0x3Cu64 << 56) | combine);
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (64u64 << 32) | u64::from(origin),
        );
        words.extend(triangle_edge(0x0A, [(4.0, 4.0), (10.0, 6.0), (4.0, 12.0)]));
        words.push(0); // s/t/w c.i
        words.push(0); // s/t/w x.i
        words.push(0); // c.f
        words.push(0x6000u64 << 48); // s.x.f (0.375)
        words.push(0); // e.i
        words.push(0); // y.i
        words.push(0); // e.f
        words.push(0x6000u64 << 32); // t.y.f (0.375)
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
        machine.run_frame().expect("frame");

        let pixel = |x: usize, y: usize| {
            let index = (y * WIDTH as usize + x) * 4;
            let frame = machine.frame();
            [
                frame[index],
                frame[index + 1],
                frame[index + 2],
                frame[index + 3],
            ]
        };
        assert_eq!(pixel(5, 5), [0xFF, 0, 0, 0xFF], "row 0 red");
        assert_eq!(pixel(5, 7), [0, 0xFF, 0, 0xFF], "row 1 green");
        assert_eq!(pixel(4, 10), [0, 0, 0xFF, 0xFF], "row 2 blue");
        assert_eq!(pixel(10, 10), [0, 0, 0, 0], "outside the triangle");
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rdp_triangles_render_non_rectangular_geometry() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        let origin = 0x2000u32;
        machine.write_register(0x0440_0004, origin);
        machine.write_register(0x0440_0008, 64);

        let mut words: Vec<u64> = Vec::new();
        words.push(
            (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (64u64 << 32) | u64::from(origin),
        );
        words.push((0x3Au64 << 56) | 0xFF00_00FF); // red
                                                   // Slanted triangle: (4,4) (12,6) (6,14).
        words.extend(triangle_edge(0x08, [(4.0, 4.0), (12.0, 6.0), (6.0, 14.0)]));
        // Slanted shaded triangle, green shade colour.
        words.extend(triangle_edge(
            0x0C,
            [(20.0, 4.0), (28.0, 6.0), (22.0, 14.0)],
        ));
        words.push((0xFF00u64 << 32) | 0xFF00); // shade: g=0xFF, a=0xFF
        words.push(0);
        words.push(0);
        words.push(0);
        for (index, word) in words.iter().enumerate() {
            machine.write_u32(index as u64 * 8, (word >> 32) as u32);
            machine.write_u32(index as u64 * 8 + 4, *word as u32);
        }
        let end = (words.len() * 8) as u32;
        machine.write_u32(0x0410_0000, 0);
        machine.write_u32(0x0410_0004, end);
        machine.run_frame().expect("frame");

        let pixel = |x: usize, y: usize| {
            let index = (y * WIDTH as usize + x) * 4;
            let frame = machine.frame();
            [
                frame[index],
                frame[index + 1],
                frame[index + 2],
                frame[index + 3],
            ]
        };
        assert_eq!(pixel(7, 9), [0xFF, 0, 0, 0xFF], "inside red triangle");
        assert_eq!(pixel(10, 10), [0, 0, 0, 0], "outside red hypotenuse");
        assert_eq!(pixel(5, 12), [0, 0, 0, 0], "outside red left edge");
        assert_eq!(pixel(23, 9), [0, 0xFF, 0, 0xFF], "inside green triangle");
        assert_eq!(pixel(26, 10), [0, 0, 0, 0], "outside green hypotenuse");
    }

    #[test]
    fn machine_watchdog_and_nmi_lines() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        machine.cpu.cp0[9] = 0xFFFF_FFFE;
        machine.cpu.cp0[11] = 0xFFFF_FFFF;
        machine.step().expect("watchdog");
        assert_eq!(machine.cpu.pc, 0xBFC0_0000);
        assert_eq!(machine.cpu.cp0[30], 0xA400_0044);

        let mut machine = Machine::new(&cart);
        machine.signal_nmi();
        machine.step().expect("NMI");
        assert_eq!(machine.cpu.pc, 0xBFC0_0000);
        assert_eq!(machine.cpu.cp0[30], 0xA400_0040);

        let mut machine = Machine::new(&cart);
        machine.signal_cache_error();
        machine.step().expect("cache error");
        assert_eq!(machine.cpu.pc, 0xA000_0000);
    }

    #[test]
    fn machine_error_level_entry() {
        let cart = cart_with(&[0x1000_FFFF, 0x0000_0000]);
        let mut machine = Machine::new(&cart);
        assert_eq!(machine.cpu.pc, 0xA400_0040);
        machine.take_error(ErrorSource::Nmi);
        assert_eq!(machine.cpu.pc, 0xBFC0_0000);
        assert_eq!(machine.cpu.cp0[30], 0xA400_0040);
        assert_ne!(machine.cpu.cp0[12] & 4, 0);
        machine.take_error(ErrorSource::Reset);
        assert_eq!(machine.cpu.pc, 0xBFC0_0000);
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
