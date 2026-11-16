//! Print the decoded SNES header and run the S-CPU for a short warm-up.

use std::env;
use std::process::ExitCode;

use unko_snes::{Cartridge, Snes};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: snes-info <rom>");
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
    system.reset();
    println!("cpu reset:   {}", system.cpu);
    let (cycles, instructions) = system.run_cycles(200_000, 50_000);
    println!("cpu after:   {}", system.cpu);
    println!("ran:         {instructions} instructions, {cycles} cycles");
    ExitCode::SUCCESS
}
