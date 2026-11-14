//! Dumps the visible nametable as text for test-ROM verdicts.
//!
//! Usage: `cargo run -p unko-nes --example screentext -- <rom> [frames]`
//!
//! Test ROMs that print with the standard ASCII-ordered font can then be
//! read without OCR. Tile indices outside the printable range are shown as
//! `?`, and tile 0 as a space.

use std::env;
use std::fs;

use unko_nes::System;

fn main() {
    let path = env::args().nth(1).expect("rom path");
    let frames: u32 = env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(120);
    let rom = fs::read(path).expect("read rom");
    let mut system = System::from_ines(&rom).expect("load");
    for _ in 0..frames {
        system.run_frame();
    }
    for row in 0..30u16 {
        let mut line = String::new();
        for column in 0..32u16 {
            let tile = system.vram_peek(0x2000 + row * 32 + column);
            line.push(match tile {
                0 => ' ',
                0x20..=0x7E => char::from(tile),
                _ => '?',
            });
        }
        println!("{line}");
    }
}
