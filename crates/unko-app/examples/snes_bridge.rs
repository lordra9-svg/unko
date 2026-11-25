//! Runs a SNES title through the same `NesCore` interface the GUI player uses
//! (`unko_app::snes::SnesAdapter`) and writes the audio the player would queue.
//!
//! Usage: snes_bridge <rom> <ipl.rom> <cx4.rom|-> <frames> <out.f32>

use std::io::Write;

use unko_app::snes::SnesAdapter;
use unko_core::LoadOptions;
use unko_nes_api::NesCore;
use unko_snes_host::FirmwareKind;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!("usage: snes_bridge <rom> <ipl.rom> <cx4.rom|-> <frames> <out.f32>");
        std::process::exit(2);
    }
    let rom = std::fs::read(&args[1]).expect("read rom");
    let ipl = std::fs::read(&args[2]).expect("read ipl");

    let mut core = SnesAdapter::new();
    core.set_firmware(FirmwareKind::Ipl, &ipl).expect("ipl");
    if args[3] != "-" {
        let cx4 = std::fs::read(&args[3]).expect("read cx4");
        core.set_firmware(FirmwareKind::Cx4, &cx4).expect("cx4");
    }

    // Drive it exactly as the player does: a `Box<dyn NesCore>`.
    let mut core: Box<dyn NesCore> = Box::new(core);
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
    let info = core.info();
    println!(
        "system={:?} core={} frames={frames} stereo={stereo} video={}x{} fps={:.3}",
        info.system,
        info.id,
        video.width,
        video.height,
        core.frame_rate()
    );
}
