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
pub mod dma;
pub mod dsp;
pub mod io;
pub mod ppu;
pub mod rom;
pub mod spc700;
pub mod system;

pub use apu::{Apu, PortEvent, PortEventKind};
pub use cpu::{Cpu, CpuBus, Flags};
pub use dma::{Dma, DmaBus, DmaChannel, DmaTransfer};
pub use dsp::Dsp;
pub use io::CpuIo;
pub use ppu::Ppu;
pub use rom::{
    region_for_country, rom_size_bytes, sram_size_bytes, BusTarget, Cartridge, Mapping, Region,
    RomError, RomHeader,
};
pub use spc700::{Flags as Spc700Flags, Spc700, SpcBus};
pub use system::{Snes, SnesState, StateError, SAVE_STATE_MAGIC, SAVE_STATE_VERSION};
