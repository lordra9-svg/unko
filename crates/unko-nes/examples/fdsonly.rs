//! Dumps the APU-muted FDS expansion channel as raw `f32` for oracle work.
//!
//! Usage: `cargo run -p unko-nes --example fdsonly -- <disk.fds> <bios> <frames> <out.f32>`
//!
//! All APU channels are muted and the FDS expansion gain is 1.0, so the file
//! is the post-resampler FDS stream alone. It is meant to be compared against
//! another emulator's expansion-only capture, not used by the frontend.

use std::fs;

use unko_nes::System;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let disk = fs::read(&args[1]).expect("read disk");
    let bios = fs::read(&args[2]).expect("read bios");
    let frames: u32 = args[3].parse().expect("frames");
    let mut system = System::from_fds(&disk, &bios).expect("load FDS");
    system.set_channel_gains([0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    let mut samples = Vec::new();
    for _ in 0..frames {
        system.run_frame();
        samples.extend_from_slice(system.audio());
    }
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
    }
    fs::write(&args[4], bytes).expect("write samples");
}
