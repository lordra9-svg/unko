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
//! The first slice is ROM container support: copier-header detection, LoROM /
//! HiROM / ExLoROM / ExHiROM layout detection, internal-header decoding, SRAM
//! sizing and cartridge bus mapping. The 65816, SPC700, PPU and DSP follow.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod rom;

pub use rom::{
    rom_size_bytes, sram_size_bytes, BusTarget, Cartridge, Mapping, RomError, RomHeader,
};
