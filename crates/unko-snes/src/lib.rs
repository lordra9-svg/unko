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

pub mod cpu;
pub mod rom;
pub mod system;

pub use cpu::{Cpu, CpuBus, Flags};
pub use rom::{
    rom_size_bytes, sram_size_bytes, BusTarget, Cartridge, Mapping, RomError, RomHeader,
};
pub use system::Snes;
