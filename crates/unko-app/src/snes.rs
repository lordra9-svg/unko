//! System dispatch for the player's core.
//!
//! The desktop player stores a `Box<dyn NesCore>` and renders whatever RGBA
//! frame the core reports. NES titles use the accuracy adapter directly; any
//! other image is handed to the SNES adapter, which implements the same
//! [`NesCore`] surface (its NES-specific device hooks are no-ops) so the player
//! loop, texture upload, audio queue and save-state path are unchanged.

use unko_core::{
    AudioBuffer, Core, CoreError, CoreInfo, FrameBuffer, FrameStats, InputState, LoadOptions,
};
use unko_nes_api::{NesCore, VideoFilter};
use unko_snes_api::{CpuState, DspState, PpuState};
use unko_snes_host::{FirmwareKind, SnesCore};

/// SNES debugger snapshots for the loaded machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnesDebugView {
    /// 65C816 registers.
    pub cpu: CpuState,
    /// S-PPU position and display state.
    pub ppu: PpuState,
    /// S-DSP register file.
    pub dsp: DspState,
}

/// iNES magic; anything else with a valid SNES header is treated as SNES.
const INES_MAGIC: &[u8] = b"NES\x1a";

/// Whether `rom` is an iNES/NES 2.0 image.
#[must_use]
pub fn is_nes_rom(rom: &[u8]) -> bool {
    rom.starts_with(INES_MAGIC)
}

/// Builds the core for the image, dispatching on the container signature.
#[must_use]
pub fn create_for_rom(rom: &[u8]) -> Box<dyn NesCore> {
    if !is_nes_rom(rom) && unko_snes_host::is_snes_rom(rom) {
        Box::new(SnesAdapter::new())
    } else {
        Box::new(unko_nes_host::AccuracyCore::new())
    }
}

/// [`NesCore`] adapter over the SNES core, used by the existing player loop.
pub struct SnesAdapter {
    inner: SnesCore,
}

impl SnesAdapter {
    /// Creates an unloaded SNES core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SnesCore::new(),
        }
    }

    /// Installs external firmware before the next load.
    pub fn set_firmware(&mut self, kind: FirmwareKind, bytes: &[u8]) -> Result<(), CoreError> {
        self.inner.set_firmware(kind, bytes)
    }

    /// The wrapped SNES core.
    #[must_use]
    pub fn inner(&self) -> &SnesCore {
        &self.inner
    }

    /// SNES debugger snapshots, when a cartridge is loaded.
    #[must_use]
    pub fn snes_debug(&self) -> Option<SnesDebugView> {
        use unko_snes_api::SnesCore as _;
        Some(SnesDebugView {
            cpu: self.inner.debug_cpu()?,
            ppu: self.inner.debug_ppu()?,
            dsp: self.inner.debug_dsp()?,
        })
    }

    /// Reads a byte from a SNES debug address space.
    #[must_use]
    pub fn snes_debug_peek(&self, space: unko_snes_api::DebugSpace, address: u32) -> Option<u8> {
        use unko_snes_api::SnesCore as _;
        self.inner.debug_peek(space, address)
    }

    /// Writes one byte into a SNES debug address space.
    pub fn snes_debug_poke(
        &mut self,
        space: unko_snes_api::DebugSpace,
        address: u32,
        value: u8,
    ) -> bool {
        use unko_snes_api::SnesCore as _;
        self.inner.debug_poke(space, address, value)
    }

    /// Assembles one 65816 instruction for `address`.
    pub fn snes_assemble(
        &self,
        text: &str,
        address: u32,
    ) -> Result<Vec<u8>, unko_snes_api::AsmError> {
        unko_snes_api::assemble(text, address)
    }

    /// Assembles one instruction and writes the bytes into a debug space.
    pub fn snes_patch(
        &mut self,
        space: unko_snes_api::DebugSpace,
        address: u32,
        text: &str,
    ) -> Result<Vec<u8>, String> {
        let bytes = self
            .snes_assemble(text, address)
            .map_err(|error| error.to_string())?;
        for (offset, byte) in bytes.iter().enumerate() {
            let target = address.wrapping_add(offset as u32);
            if !self.snes_debug_poke(space, target, *byte) {
                return Err(format!("write failed at {target:06X}"));
            }
        }
        Ok(bytes)
    }

    /// Executes one SNES CPU instruction.
    pub fn snes_debug_step(&mut self) -> Option<u32> {
        use unko_snes_api::SnesCore as _;
        self.inner.debug_step().ok()
    }

    /// Disassembles one instruction at a 24-bit address.
    pub fn snes_disassemble(&mut self, address: u32) -> Option<unko_snes_api::Disassembly> {
        use unko_snes_api::SnesCore as _;
        self.inner.debug_disassemble(address)
    }

    /// Disassembles up to `count` consecutive instructions.
    pub fn snes_listing(&mut self, address: u32, count: usize) -> Vec<unko_snes_api::Disassembly> {
        let mut rows = Vec::with_capacity(count);
        let mut pc = address;
        for _ in 0..count {
            let Some(row) = self.snes_disassemble(pc) else {
                break;
            };
            pc = pc.wrapping_add(row.bytes.len().max(1) as u32);
            rows.push(row);
        }
        rows
    }
}

impl Default for SnesAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Core for SnesAdapter {
    fn info(&self) -> CoreInfo {
        self.inner.info()
    }

    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError> {
        self.inner.load(rom, options)
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        self.inner.reset()
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        self.inner.run_frame()
    }

    fn video(&self) -> FrameBuffer<'_> {
        self.inner.video()
    }

    fn audio(&self) -> AudioBuffer<'_> {
        self.inner.audio()
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        self.inner.set_input(port, input);
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.inner.set_sample_rate(sample_rate);
    }

    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        self.inner.save_state(out)
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        self.inner.load_state(state)
    }

    fn sram(&mut self) -> &[u8] {
        self.inner.sram()
    }

    fn load_sram(&mut self, sram: &[u8]) -> Result<(), CoreError> {
        self.inner.load_sram(sram)
    }
}

impl NesCore for SnesAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn set_video_filter(&mut self, _filter: VideoFilter) {}

    fn frame_rate(&self) -> f64 {
        unko_snes_api::SnesCore::frame_rate(&self.inner)
    }

    fn as_core(&mut self) -> &mut dyn Core {
        self
    }

    fn as_core_ref(&self) -> &dyn Core {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_lorom() -> Vec<u8> {
        let mut rom: Vec<u8> = (0..0x8000u32)
            .map(|index| (index.wrapping_mul(37) & 0xFF) as u8)
            .collect();
        let offset = 0x7FC0;
        rom[offset..offset + 21].fill(0);
        rom[offset..offset + 21].copy_from_slice(b"TEST ROM             ");
        rom[offset + 0x15] = 0x20;
        rom[offset + 0x16] = 0x02;
        rom[offset + 0x17] = 5;
        rom[offset + 0x18] = 0x03;
        rom[offset + 0x19] = 1;
        rom[offset + 0x1A] = 0x01;
        rom[offset + 0x3C..offset + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
        let mut checksum_source = rom.clone();
        checksum_source[offset + 0x1C..offset + 0x20].fill(0);
        let base: u32 = checksum_source.iter().map(|&b| u32::from(b)).sum();
        let checksum = ((base + 0x01FE) & 0xFFFF) as u16;
        rom[offset + 0x1C..offset + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        rom[offset + 0x1E..offset + 0x20].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    #[test]
    fn dispatches_on_the_container_signature() {
        assert!(is_nes_rom(b"NES\x1a\x02\x01"));
        assert!(!is_nes_rom(&[0u8; 16]));

        let nes = create_for_rom(b"NES\x1a\x02\x01");
        assert_eq!(nes.info().system, unko_core::SystemId::Nes);

        // A valid SNES cartridge header dispatches to the SNES adapter.
        let snes = create_for_rom(&build_lorom());
        assert_eq!(snes.info().system, unko_core::SystemId::Snes);

        // Noise is not a valid cartridge for either system; the player defaults
        // to the NES core (its upstream loader is NES-only).
        let noise = create_for_rom(&[0u8; 32]);
        assert_eq!(noise.info().system, unko_core::SystemId::Nes);
    }
}
