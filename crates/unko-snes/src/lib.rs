//! Standalone Super Nintendo / Super Famicom core.
//!
//! This crate is intentionally independent of the Unko frontend and of
//! [`unko_core`](https://docs.rs/unko-core). It is being built as a clean
//! Rust core in the same shape as `unko-nes`; a future `unko-snes-host` crate
//! will provide the optional `unko_core::Core` adapter.
//!
//! The core is written from public hardware documentation and the ISC-licensed
//! ares source tree. MesenCE is a reference oracle only and RustySNES is an
//! Apache-2.0 oracle; neither is translated into this crate. See
//! `PROVENANCE.md` and `docs/accuracy.md`.
//!
//! # Current scope
//!
//! The core supports ROM container parsing (copier-header detection, LoROM /
//! HiROM / ExLoROM / ExHiROM layout detection, header decoding, SRAM sizing and
//! cartridge bus mapping), a cycle-stepped 65C816 and a minimal S-CPU memory
//! map. The SPC700, PPU and DSP follow.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod apu;
pub mod cpu;
pub mod cx4;
pub mod dma;
pub mod dsp;
pub mod epsonrtc;
pub mod gsu;
pub mod io;
pub mod necdsp;
pub mod ppu;
pub mod rom;
pub mod sa1;
pub mod sdd1;
pub mod spc700;
pub mod spc7110;
pub mod system;

pub use apu::{Apu, PortEvent, PortEventKind};
pub use cpu::{Cpu, CpuBus, Flags};
pub use cx4::{Cx4, Cx4Error, Cx4Window};
pub use dma::{Dma, DmaBus, DmaChannel, DmaTransfer};
pub use dsp::Dsp;
pub use epsonrtc::EpsonRtc;
pub use gsu::Gsu;
pub use io::CpuIo;
pub use necdsp::{DspError, DspFlags, DspRegisters, DspWindow, NecDsp};
pub use ppu::Ppu;
pub use rom::{
    region_for_country, rom_size_bytes, sram_size_bytes, BusTarget, Cartridge, Mapping, Region,
    RomError, RomHeader,
};
pub use sa1::Sa1;
pub use sdd1::{Sdd1, Sdd1Banks, Sdd1Dma};
pub use spc700::{Flags as Spc700Flags, Spc700, SpcBus};
pub use spc7110::{Spc7110, Spc7110Decompressor};
pub use system::{Snes, SnesState, StateError, SAVE_STATE_MAGIC, SAVE_STATE_VERSION};
