//! Repair known Vs. System iNES headers into NES 2.0 images.

use std::fs;
use std::path::{Path, PathBuf};

struct Spec {
    name: &'static str,
    output: &'static str,
    ppu: u8,
    cabinet: u8,
    mapper: Option<u16>,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "VS Castlevania.nes",
        output: "VS Castlevania.nes",
        ppu: 3,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Clu Clu Land.nes",
        output: "VS Clu Clu Land.nes",
        ppu: 5,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Duck Hunt Hack.nes",
        output: "Vs. Duck Hunt (hack).nes",
        ppu: 0,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Excitebike Hack.nes",
        output: "Vs. Excitebike (hack).nes",
        ppu: 4,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Excitebike.nes",
        output: "VS Excitebike.nes",
        ppu: 4,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Gradius Hack.nes",
        output: "Vs. Gradius (hack).nes",
        ppu: 2,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Ice Climber Hack.nes",
        output: "Vs. Ice Climber (hack).nes",
        ppu: 5,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Pinball Hack.nes",
        output: "Vs. Pinball (hack).nes",
        ppu: 2,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Super Mario Bros Version MAME V1.1A.nes",
        output: "VS Super Mario Bros Version MAME V1.1A.nes",
        ppu: 5,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS TKO Boxing Hack.nes",
        output: "Vs. TKO Boxing (hack).nes",
        ppu: 4,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS The Goonies Hack.nes",
        output: "Vs. The Goonies (hack).nes",
        ppu: 4,
        cabinet: 0,
        mapper: Some(75),
    },
    Spec {
        name: "VS. Gumshoe (VS) [a1].nes",
        output: "Vs. Gumshoe.nes",
        ppu: 0,
        cabinet: 0,
        mapper: None,
    },
    Spec {
        name: "VS Castlevania Hack.nes",
        output: "Vs. Castlevania (hack).nes",
        ppu: 3,
        cabinet: 0,
        mapper: Some(2),
    },
];

fn parse_args(args: &[String]) -> Result<(PathBuf, PathBuf, bool), String> {
    let mut source = None;
    let mut output = PathBuf::from("test-roms/vs");
    let mut all = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                output = PathBuf::from(args.get(index).ok_or("--output needs a path")?);
            }
            "--all" => all = true,
            other => source = Some(PathBuf::from(other)),
        }
        index += 1;
    }
    Ok((source.ok_or("source directory is required")?, output, all))
}

fn repaired(data: &[u8], spec: &Spec) -> Result<Vec<u8>, String> {
    if data.len() < 16 {
        return Err("file is too short for an iNES header".into());
    }
    let has_header = &data[..4] == b"NES\x1a";
    let (mut mapper, body) = if has_header {
        let flags6 = data[6];
        let flags7 = data[7];
        let mut mapper = u16::from(flags6 >> 4) | u16::from(flags7 & 0xF0);
        if flags7 & 0x0C == 0x08 {
            mapper |= u16::from(data[8] & 0x0F) << 8;
        }
        (mapper, data[16..].to_vec())
    } else {
        let body = if data.len() % 16384 == 16 {
            data[16..].to_vec()
        } else {
            data.to_vec()
        };
        (0, body)
    };
    if let Some(forced) = spec.mapper {
        mapper = forced;
    }
    if mapper > 0xFF {
        return Err("only mappers 0-255 are supported".into());
    }
    let prg = if has_header {
        data[4]
    } else {
        u8::try_from(body.len() / 16384).map_err(|_| "PRG image is too large")?
    };
    let chr = if has_header { data[5] } else { 0 };
    let mut header = [0u8; 16];
    header[..4].copy_from_slice(b"NES\x1a");
    header[4] = prg;
    header[5] = chr;
    header[6] = ((mapper & 0x0F) as u8) << 4 | 0x08;
    header[7] = (mapper & 0xF0) as u8 | 0x08 | 0x01;
    header[8] = ((mapper >> 8) & 0x0F) as u8;
    header[13] = (spec.cabinet << 4) | (spec.ppu & 0x0F);
    let mut out = header.to_vec();
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (source, output, all) = parse_args(args)?;
    fs::create_dir_all(&output).map_err(|err| format!("{}: {err}", output.display()))?;
    let mut staged = 0;
    for spec in SPECS {
        let path = source.join(spec.name);
        if !path.is_file() {
            if all {
                println!("skip missing {}", spec.name);
            }
            continue;
        }
        let bytes = fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let image = repaired(&bytes, spec)?;
        let target = output.join(spec.output);
        fs::write(&target, image).map_err(|err| format!("{}: {err}", target.display()))?;
        println!(
            "staged {} (PPU {}, cabinet {})",
            target.display(),
            spec.ppu,
            spec.cabinet
        );
        staged += 1;
    }
    if staged == 0 {
        return Err(format!(
            "no known Vs. images found under {}",
            source.display()
        ));
    }
    println!("{staged} file(s) staged in {}", output.display());
    Ok(())
}

#[allow(dead_code)]
fn is_nes(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"NES\x1a"))
}
