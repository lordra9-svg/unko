//! Adapter smoke test: run a ROM through the `unko-core` interface and write the
//! audio the host would receive as interleaved f32.
//!
//! Usage: audio <rom> <ipl.rom> <cx4.rom|-> <frames> <out.f32>

use std::io::Write;

use unko_core::{Core, LoadOptions};
use unko_snes_host::{FirmwareKind, SnesCore};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!("usage: audio <rom> <ipl.rom> <cx4.rom|-> <frames> <out.f32>");
        std::process::exit(2);
    }
    let rom = std::fs::read(&args[1]).expect("read rom");
    let ipl = std::fs::read(&args[2]).expect("read ipl");

    let mut core = SnesCore::new();
    core.set_firmware(FirmwareKind::Ipl, &ipl).expect("ipl");
    if args[3] != "-" {
        let cx4 = std::fs::read(&args[3]).expect("read cx4");
        core.set_firmware(FirmwareKind::Cx4, &cx4).expect("cx4");
    }
    core.load(&rom, &LoadOptions::default()).expect("load");

    let frames: u32 = args[4].parse().expect("frames");
    let mut out = std::fs::File::create(&args[5]).expect("create out");
    let mut stereo = 0usize;
    for _ in 0..frames {
        let stats = core.run_frame().expect("run frame");
        stereo += stats.samples;
        let mut bytes = Vec::with_capacity(core.audio().samples.len() * 4);
        for sample in core.audio().samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        out.write_all(&bytes).expect("write");
    }
    let video = core.video();
    println!(
        "frames={frames} stereo_frames={stereo} video={}x{}",
        video.width, video.height
    );
}
