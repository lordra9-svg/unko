//! Print the decoded SNES header and run the S-CPU for a short warm-up.

use std::env;
use std::path::PathBuf;
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
    let cartridge = match Cartridge::from_bytes_lenient(&bytes) {
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
    if let Some(path) = std::env::var_os("DSP1_ROM") {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("{}: {error}", path.to_string_lossy());
                return ExitCode::FAILURE;
            }
        };
        if let Err(error) = system.set_dsp1_firmware(&bytes) {
            eprintln!("{}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
        println!("dsp1:        {}", path.to_string_lossy());
    }
    if let Some(path) = std::env::var_os("CX4_ROM") {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("{}: {error}", path.to_string_lossy());
                return ExitCode::FAILURE;
            }
        };
        if let Err(error) = system.set_cx4_firmware(&bytes) {
            eprintln!("{}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
        println!("cx4:         {}", path.to_string_lossy());
    }
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
    let joypad0 = std::env::var("JOYPAD0")
        .ok()
        .and_then(|bits| u16::from_str_radix(bits.trim_start_matches("0x"), 16).ok());
    let joypad0_from: u32 = std::env::var("JOYPAD0_FROM")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if let Some(value) = joypad0 {
        if joypad0_from == 0 {
            system.set_joypad(0, value);
        }
    }
    if std::env::var_os("PORT_OUT").is_some() || std::env::var_os("SPC_TRACE").is_some() {
        system.apu.set_port_logging(true);
    }
    system.reset();
    if let Some(path) = std::env::var_os("STATE_LOAD") {
        let bytes = std::fs::read(&path).expect("read STATE_LOAD");
        system.load_state(&bytes).expect("load save state");
        println!("state load:  {}", path.to_string_lossy());
    }
    if std::env::var_os("DSP_TRACE_OUT").is_some() {
        system.apu.dsp_mut().enable_trace();
    }
    println!("cpu reset:   {}", system.cpu);
    println!("spc reset:   {}", system.apu.spc);
    // Optional `PC_WATCH=8000,8195` breakpoint log (debug).
    let watched_pcs: Vec<u16> = std::env::var("PC_WATCH")
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| u16::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok())
        .collect();
    let mut audio_out = std::env::var_os("AUDIO_OUT")
        .map(|path| std::fs::File::create(path).expect("create AUDIO_OUT"));
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
            if let Some(value) = joypad0 {
                system.set_joypad(0, if frames >= joypad0_from { value } else { 0 });
            }
            if watched_pcs.contains(&system.cpu.pc) {
                eprintln!(
                        "WATCH {:02X}:{:04X} clock={} frame={frames} A={:04X} X={:04X} Y={:04X} D={:04X} db={:02X} dp12={:02X}{:02X}{:02X}",
                        system.cpu.pbr,
                        system.cpu.pc,
                        system.clock,
                        system.cpu.a,
                        system.cpu.x,
                        system.cpu.y,
                        system.cpu.d,
                        system.cpu.dbr,
                        system.wram()[0x14],
                        system.wram()[0x13],
                    system.wram()[0x12]
                );
            }
            if let Some(path) = std::env::var_os("PC_TRACE") {
                let from: u64 = std::env::var("PC_TRACE_FROM")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let to: u64 = std::env::var("PC_TRACE_TO")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(u64::MAX);
                if (from..=to).contains(&system.clock) {
                    let line = format!(
                        "{} {:02X}:{:04X}\n",
                        system.clock, system.cpu.pbr, system.cpu.pc
                    );
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
                }
            }
            cycles += u64::from(system.step());
            instructions += 1;
            if let Some(file) = audio_out.as_mut() {
                use std::io::Write;
                let mut bytes = Vec::new();
                for (left, right) in system.apu.drain_samples() {
                    bytes.extend_from_slice(&(f32::from(left) / 32768.0).to_le_bytes());
                    bytes.extend_from_slice(&(f32::from(right) / 32768.0).to_le_bytes());
                }
                let _ = file.write_all(&bytes);
            }
            if system.cpu.stp && std::env::var_os("STP_LOG").is_some() {
                eprintln!(
                    "STP at {:02X}:{:04X} frame={frames} master={} line={}",
                    system.cpu.pbr,
                    system.cpu.pc,
                    system.clock,
                    system.ppu.line()
                );
            }
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
    if let Some(path) = std::env::var_os("STATE_SAVE") {
        let bytes = system.save_state();
        std::fs::write(&path, &bytes).expect("write STATE_SAVE");
        println!(
            "state save:  {} ({} bytes)",
            path.to_string_lossy(),
            bytes.len()
        );
    }
    if std::env::var_os("CX4_STATE").is_some() {
        if let Some(cx4) = &system.cx4 {
            eprintln!(
                "CX4 pc={:02X} pb={:04X} halt={} cache={} dma={} busy={} ticks={} a={:06X} mar={:06X} dpr={:06X} gpr={:06X?}",
                cx4.regs.pc,
                cx4.regs.pb,
                cx4.io.halt,
                cx4.io.cache.enable,
                cx4.io.dma.enable,
                cx4.busy(),
                cx4.ticks,
                cx4.regs.a,
                cx4.regs.mar,
                cx4.regs.dpr,
                cx4.regs.gpr
            );
            eprintln!(
                "CX4 cache base={:06X} pb={:04X} pc={:02X} page={} preload={} wait={:?} irq={} vec0={:02X?}",
                cx4.io.cache.base,
                cx4.io.cache.pb,
                cx4.io.cache.pc,
                cx4.io.cache.page,
                cx4.io.cache.preload,
                cx4.io.wait,
                cx4.io.irq,
                &cx4.io.vector[..8]
            );
        } else {
            eprintln!("CX4 not attached");
        }
    }
    if std::env::var_os("SA1_STATE").is_some() {
        if let Some(sa1) = &system.sa1 {
            eprintln!(
                "SA1 pc={:02X}:{:04X} s={:04X} p={:02X} e={} ticks={} resb={} rdyb={} pending={} hc={} vc={} mr={:010X} sbm={:02X} cbm={:02X} sw46={} swen={} cwen={} bwp={:X} bbf={}",
                sa1.cpu.pbr,
                sa1.cpu.pc,
                sa1.cpu.s,
                sa1.cpu.p(),
                sa1.cpu.emulation,
                sa1.ticks,
                sa1.io.sa1_resb,
                sa1.io.sa1_rdyb,
                sa1.interrupt_pending,
                sa1.hcounter,
                sa1.vcounter,
                sa1.io.mr,
                sa1.io.sbm,
                sa1.io.cbm,
                sa1.io.sw46,
                sa1.io.swen,
                sa1.io.cwen,
                sa1.io.bwp,
                sa1.io.bbf
            );
            eprintln!(
                "SA1 iram[6DE..6E2]={:02X?} dma_count={} dtc={:04X} dmaen={}",
                &sa1.iram[0x6DE..0x6E2],
                sa1.dma_count,
                sa1.io.dtc,
                sa1.io.dmaen
            );
        } else {
            eprintln!("SA1 not attached");
        }
    }
    if std::env::var_os("SDD1_STATE").is_some() {
        if let Some(sdd1) = &system.sdd1 {
            eprintln!(
                "SDD1 r4800={:02X} r4801={:02X} banks={:02X},{:02X},{:02X},{:02X} dma_ready={} streams={} bytes={} dma={:02X?}",
                sdd1.r4800,
                sdd1.r4801,
                sdd1.banks.bank0,
                sdd1.banks.bank1,
                sdd1.banks.bank2,
                sdd1.banks.bank3,
                sdd1.dma_ready,
                sdd1.streams,
                sdd1.bytes,
                sdd1.dma
            );
        } else {
            eprintln!("SDD1 not attached");
        }
    }
    if std::env::var_os("DSP_STATE").is_some() {
        if let Some(dsp) = &system.dsp1 {
            eprintln!(
                "DSP pc={:04X} sr={:04X} dr={:04X} rp={:04X} dp={:04X} k={} l={} ticks={}",
                dsp.regs.pc,
                dsp.status(),
                dsp.regs.dr,
                dsp.regs.rp,
                dsp.regs.dp,
                dsp.regs.k,
                dsp.regs.l,
                dsp.ticks
            );
        } else {
            eprintln!("DSP not attached");
        }
    }
    if std::env::var_os("PPU_STATE").is_some() {
        eprintln!(
            "PPU overscan={} interlace={} hires={} height={} width={} vdisp={} last={}",
            system.ppu.frame_overscan(),
            system.ppu.frame_interlace(),
            system.ppu.frame_hires(),
            system.ppu.frame_height(),
            system.ppu.frame_width(),
            system.ppu.vdisp(),
            system.ppu.visible_last(),
        );
        let (mode, tm, ts, sc, nba, bh, bv) = system.ppu.debug_regs();
        eprintln!(
            "PPU mode={mode} tm={tm:02X} ts={ts:02X} sc={sc:02X?} nba={nba:02X?} h={bh:?} v={bv:?}"
        );
    }
    println!(
        "cpu after:   {}  nmi_services={} irq_services={}",
        system.cpu, system.cpu.nmi_services, system.cpu.irq_services
    );
    println!("spc after:   {}", system.apu.spc);
    println!(
        "apu ports:   cpu<-apu {:02X?}  apu<-cpu {:02X?}",
        [0, 1, 2, 3].map(|port| system.apu.cpu_read(port)),
        [0, 1, 2, 3].map(|port| system.apu.apu_port(port)),
    );
    if let Some(path) = std::env::var_os("PORT_OUT") {
        let mut out = String::new();
        for event in system.apu.debug.events() {
            out.push_str(&format!(
                "{} {} {} {:02X} {:04X} {}\n",
                event.clock,
                match event.kind {
                    unko_snes::PortEventKind::CpuWrite => "W",
                    unko_snes::PortEventKind::CpuRead => "CR",
                    unko_snes::PortEventKind::SmpRead => "R",
                    unko_snes::PortEventKind::SmpWrite => "S",
                },
                event.port,
                event.value,
                event.spc_pc,
                event.spc_instruction
            ));
        }
        let _ = std::fs::write(path, out);
    }
    if let Some(path) = std::env::var_os("SPC_TRACE") {
        let mut out = String::new();
        for (index, (clock, pc, clocks)) in system.apu.debug.trace().iter().enumerate() {
            out.push_str(&format!("{index} {clock} {pc:04X} {clocks}\n"));
        }
        let _ = std::fs::write(path, out);
    }
    if let Some(path) = std::env::var_os("DSP_TRACE_OUT") {
        let base = PathBuf::from(&path);
        if let Some(trace) = system.apu.dsp().trace.as_ref() {
            let mut out = String::new();
            for row in &trace.rows {
                out.push_str(&row.map(|v| v.to_string()).join(" "));
                out.push('\n');
            }
            let _ = std::fs::write(path, out);
            let mut wout = String::new();
            for (sample, address, value) in &trace.writes {
                wout.push_str(&format!("{sample} {address:02X} {value:02X}\n"));
            }
            let wpath = std::env::var_os("DSP_WRITES_OUT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}.writes", base.to_string_lossy())));
            let _ = std::fs::write(wpath, wout);
        }
    }
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
    if std::env::var_os("AUDIO_DEBUG").is_some() {
        let dsp = system.apu.dsp();
        eprintln!(
            "dsp: mvoll={:02X} mvolr={:02X} evoll={:02X} evolr={:02X} kon={:02X} koff={:02X} flg={:02X} endx={:02X}",
            dsp.read(0x0C),
            dsp.read(0x1C),
            dsp.read(0x2C),
            dsp.read(0x3C),
            dsp.read(0x4C),
            dsp.read(0x5C),
            dsp.read(0x6C),
            dsp.read(0x7C)
        );
        for voice in 0..8u8 {
            let base = voice << 4;
            eprintln!(
                "v{voice}: volL={:02X} volR={:02X} pitch={:02X}{:02X} src={:02X} adsr0={:02X} adsr1={:02X} gain={:02X} envx={:02X} outx={:02X}",
                dsp.read(base),
                dsp.read(base + 1),
                dsp.read(base + 3),
                dsp.read(base + 2),
                dsp.read(base + 4),
                dsp.read(base + 5),
                dsp.read(base + 6),
                dsp.read(base + 7),
                dsp.read(base + 8),
                dsp.read(base + 9)
            );
        }
    }
    if std::env::var_os("AUDIO_DEBUG").is_some() {
        eprintln!(
            "timers: latches={:?} t0={:?} t1={:?} t2={:?}",
            system.apu.timer_latches(),
            system.apu.timer_state(0),
            system.apu.timer_state(1),
            system.apu.timer_state(2)
        );
    }
    if let Some(path) = std::env::var_os("DSP_OUT") {
        let dsp = system.apu.dsp();
        let mut bytes = Vec::with_capacity(0x80);
        for address in 0..0x80u8 {
            bytes.push(dsp.read(address));
        }
        let _ = std::fs::write(path, bytes);
    }
    if let Some(path) = std::env::var_os("WRAM_OUT") {
        let _ = std::fs::write(path, system.wram());
    }
    if let Some(path) = std::env::var_os("FRAME_OUT") {
        if let Err(error) = std::fs::write(&path, system.ppu.frame_buffer()) {
            eprintln!("{}: {error}", path.to_string_lossy());
        }
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
    if std::env::var_os("PPU_DEBUG").is_some() {
        eprintln!(
            "ppu regs: mode={} mosaic={:02X} sc={:02X?} nba={:02X?} tm={:02X} ts={:02X} m7={:02X} scroll={:?}",
            system.ppu.bg_mode(),
            system.ppu.mosaic(),
            [0, 1, 2, 3].map(|i| system.ppu.bg_sc(i)),
            [0, 1, 2, 3].map(|i| system.ppu.bg_nba(i)),
            system.ppu.main_screen(),
            system.ppu.sub_screen(),
            system.ppu.m7sel(),
            [0, 1, 2, 3].map(|i| system.ppu.bg_scroll(i)),
        );
    }
    println!(
        "spc inst:    {}  spc units: {}  master clock: {}  nmi_vec: {}\nppu:         brightness={} nmi={} nmi_count={} irq={} line={} vblank={} frame_ready={} nonzero_pixels={} rgb_sum={checksum}",
        system.apu.instructions,
        system.apu.units,
        system.clock,
        system.cpu.nmi_services,
        system.ppu.display_brightness(),
        system.nmi_enabled(),
        system.nmi_count,
        system.cpu.irq_services,
        system.ppu.line(),
        system.ppu.in_vblank(),
        system.ppu.frame_ready(),
        nonzero,
    );
    println!("ran:         {instructions} instructions, {cycles} cycles");
    ExitCode::SUCCESS
}
