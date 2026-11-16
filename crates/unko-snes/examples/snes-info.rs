//! Print the decoded SNES header and run the S-CPU for a short warm-up.

use std::env;
use std::process::ExitCode;

use unko_snes::{Cartridge, Snes};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: snes-info <rom> [ipl.rom]");
        return ExitCode::FAILURE;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let cartridge = match Cartridge::from_bytes(&bytes) {
        Ok(cartridge) => cartridge,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let header = cartridge.header();
    println!("title:       {}", header.title);
    println!("mapping:     {} ({:#04X})", header.mapping, header.map_mode);
    println!(
        "type:        {} ({:#04X})",
        header.cartridge_type_name(),
        header.cartridge_type
    );
    println!("region:      {}", header.country_name());
    println!("rom size:    {:?}", header.declared_rom_size);
    println!("sram size:   {:?}", header.declared_sram_size);
    println!(
        "checksum:    {:04X} (computed {:04X})",
        header.checksum,
        header.computed_checksum(cartridge.rom())
    );
    println!("reset:       {:04X}", header.reset_vector);
    println!("header byte: {:02X}", header.header_score);
    println!("copier head: {}", cartridge.has_copier_header());

    let mut system = Snes::new(cartridge);
    if let Some(ipl_path) = env::args().nth(2) {
        let bytes = match std::fs::read(&ipl_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("{ipl_path}: {error}");
                return ExitCode::FAILURE;
            }
        };
        if bytes.len() != 64 {
            eprintln!("{ipl_path}: expected a 64-byte IPL ROM");
            return ExitCode::FAILURE;
        }
        let mut ipl = [0u8; 64];
        ipl.copy_from_slice(&bytes);
        system.set_ipl(ipl);
        println!("ipl:         {ipl_path}");
    }
    system.reset();
    println!("cpu reset:   {}", system.cpu);
    println!("spc reset:   {}", system.apu.spc);
    let (cycles, instructions) = if std::env::var_os("TRACE").is_some() {
        let mut cycles = 0u64;
        let mut instructions = 0u32;
        while instructions < 5_000_000 && !system.cpu.stp {
            cycles += u64::from(system.step());
            instructions += 1;
            if instructions.is_multiple_of(500_000) {
                eprintln!(
                    "i={instructions} cyc={cycles} {} spc={} line={} ports={:02X?}",
                    system.cpu,
                    system.apu.spc,
                    system.ppu.line(),
                    [0, 1, 2, 3].map(|p| system.apu.apu_port(p))
                );
            }
        }
        (cycles, instructions)
    } else {
        system.run_cycles(20_000_000, 5_000_000)
    };
    println!("cpu after:   {}", system.cpu);
    println!("spc after:   {}", system.apu.spc);
    println!(
        "apu ports:   cpu<-apu {:02X?}  apu<-cpu {:02X?}",
        [0, 1, 2, 3].map(|port| system.apu.cpu_read(port)),
        [0, 1, 2, 3].map(|port| system.apu.apu_port(port)),
    );
    let frame = system.ppu.frame_buffer();
    let pixels = frame.as_chunks::<4>().0;
    let nonzero = pixels
        .iter()
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count();
    let checksum: u64 = pixels
        .iter()
        .map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]))
        .sum();
    println!(
        "ppu:         line={} vblank={} frame_ready={} nonzero_pixels={} rgb_sum={checksum}",
        system.ppu.line(),
        system.ppu.in_vblank(),
        system.ppu.frame_ready(),
        nonzero,
    );
    eprintln!("spc ram 0540: {:02X?}", &system.apu.ram()[0x540..0x560]);
    println!("ran:         {instructions} instructions, {cycles} cycles");
    ExitCode::SUCCESS
}
