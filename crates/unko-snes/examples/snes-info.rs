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
    let target_frames: u32 = std::env::var("FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let (cycles, instructions) = if target_frames > 0 {
        let mut cycles = 0u64;
        let mut instructions = 0u32;
        let mut frames = 0u32;
        let instruction_limit = target_frames.saturating_mul(20_000).max(20_000_000);
        while frames < target_frames && instructions < instruction_limit && !system.cpu.stp {
            cycles += u64::from(system.step());
            instructions += 1;
            if system.ppu.frame_ready() {
                system.ppu.acknowledge_frame();
                frames += 1;
                if let Some(path) = std::env::var_os("SCROLL_LOG") {
                    let mut line = format!("{frames}");
                    for layer in 0..4 {
                        let (h, v) = system.ppu.bg_scroll(layer);
                        line.push_str(&format!(",{h},{v}"));
                    }
                    line.push('\n');
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
                }
            }
        }
        eprintln!("frames={frames}");
        (cycles, instructions)
    } else if std::env::var_os("TRACE").is_some() {
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
    if let Some(path) = std::env::var_os("DUMP_APU") {
        let _ = std::fs::write(path, system.apu.ram());
    }
    if let Ok(mask) = std::env::var("ONLY_TM") {
        if let Ok(mask) = mask.parse::<u8>() {
            system.ppu.set_layer_mask(Some(mask));
            system.ppu.acknowledge_frame();
            let mut guard = 0;
            while !system.ppu.frame_ready() && guard < 2_000_000 {
                system.step();
                guard += 1;
            }
        }
    }
    if let Some(path) = std::env::var_os("FRAME_OUT") {
        let _ = std::fs::write(path, system.ppu.frame_buffer());
    }
    if std::env::var_os("OAM_DUMP").is_some() {
        let oam = system.ppu.oam();
        for n in 0..128 {
            let y = oam[n * 4 + 1];
            if y < 224 {
                let high = oam[0x200 + n / 4];
                let x = u16::from(oam[n * 4]) | (u16::from((high >> ((n & 3) * 2)) & 1) << 8);
                let size = (high >> ((n & 3) * 2 + 1)) & 1;
                let obsel = system.ppu.obsel();
                let base = usize::from((obsel >> 5) & 7);
                let widths = [8u16, 8, 8, 16, 16, 32, 16, 16];
                let heights = [8u16, 8, 8, 16, 16, 32, 32, 32];
                let large_w = [16u16, 32, 64, 32, 64, 64, 32, 32];
                let large_h = [16u16, 32, 64, 32, 64, 64, 64, 32];
                let (w, h) = if size != 0 {
                    (large_w[base], large_h[base])
                } else {
                    (widths[base], heights[base])
                };
                eprintln!(
                    "OAM {n:3} x={x:3} y={y:3} tile={:02X} attr={:02X} size={size} {w}x{h} obsel={obsel:02X}",
                    oam[n * 4 + 2],
                    oam[n * 4 + 3]
                );
            }
        }
    }
    if let Some(path) = std::env::var_os("CGRAM_OUT") {
        let mut cgram = Vec::with_capacity(512);
        for address in 0..256u16 {
            cgram.extend_from_slice(&system.ppu.cgram_word(address as u8).to_le_bytes());
        }
        let _ = std::fs::write(path, cgram);
    }
    if let Some(path) = std::env::var_os("VRAM_OUT") {
        let mut vram = Vec::with_capacity(0x8000 * 2);
        for address in 0..0x8000u16 {
            vram.extend_from_slice(&system.ppu.vram_word(address).to_le_bytes());
        }
        let _ = std::fs::write(path, vram);
    }
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
        "ppu:         brightness={} line={} vblank={} frame_ready={} nonzero_pixels={} rgb_sum={checksum}",
        system.ppu.display_brightness(),
        system.ppu.line(),
        system.ppu.in_vblank(),
        system.ppu.frame_ready(),
        nonzero,
    );
    println!("ran:         {instructions} instructions, {cycles} cycles");
    ExitCode::SUCCESS
}
