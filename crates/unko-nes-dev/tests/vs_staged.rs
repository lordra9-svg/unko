//! Optional smoke test for locally staged Vs. System images.
//!
//! The images are not committed; `scripts/repair_vs_headers.py` stages them
//! under `test-roms/vs/`. The test skips cleanly when that folder is absent.

use std::path::Path;

use unko_nes::{Mirroring, PpuModel, System, VsSystemType};

/// `(filename, mapper, expected PPU)` for the repaired local drop.
const CASES: &[(&str, u16, PpuModel)] = &[
    ("VS Castlevania.nes", 2, PpuModel::Ppu2C04B),
    ("Vs. Castlevania (hack).nes", 2, PpuModel::Ppu2C04B),
    ("VS Clu Clu Land.nes", 99, PpuModel::Ppu2C04D),
    ("Vs. Duck Hunt (hack).nes", 1, PpuModel::Ppu2C03),
    ("VS Excitebike.nes", 99, PpuModel::Ppu2C04C),
    ("Vs. Excitebike (hack).nes", 99, PpuModel::Ppu2C04C),
    ("Vs. Gradius (hack).nes", 4, PpuModel::Ppu2C04A),
    ("Vs. Ice Climber (hack).nes", 1, PpuModel::Ppu2C04D),
    ("Vs. Pinball (hack).nes", 3, PpuModel::Ppu2C04A),
    (
        "VS Super Mario Bros Version MAME V1.1A.nes",
        1,
        PpuModel::Ppu2C04D,
    ),
    ("Vs. TKO Boxing (hack).nes", 4, PpuModel::Ppu2C04C),
    ("Vs. The Goonies (hack).nes", 75, PpuModel::Ppu2C04C),
    ("Vs. Gumshoe.nes", 99, PpuModel::Ppu2C03),
];

#[test]
fn staged_vs_images_boot_with_vs_metadata() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms/vs");
    if !root.is_dir() {
        eprintln!("skipping: {} is absent", root.display());
        return;
    }
    let mut checked = 0;
    for (name, mapper, ppu) in CASES {
        let path = root.join(name);
        if !path.exists() {
            eprintln!("skipping missing {name}");
            continue;
        }
        let rom = std::fs::read(&path).expect("read staged ROM");
        let mut system = System::from_ines(&rom).expect("load staged ROM");
        assert!(system.is_vs_system(), "{name}: VS console type");
        assert_eq!(system.cart.mapper(), *mapper, "{name}: mapper");
        assert_eq!(system.ppu.ppu_model(), *ppu, "{name}: PPU model");
        assert_eq!(
            system.vs_system_type(),
            VsSystemType::UniSystem,
            "{name}: cabinet type"
        );
        assert_eq!(
            system.cart.mirroring(),
            Mirroring::FourScreen,
            "{name}: four-screen nametables"
        );
        for _ in 0..10 {
            system.run_frame();
        }
        assert!(!system.video().is_empty(), "{name}: rendered a frame");
        checked += 1;
    }
    assert!(checked > 0, "no staged Vs. images were found");
}

#[test]
fn staged_gumshoe_switches_the_outer_prg_bank() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms/vs/Vs. Gumshoe.nes");
    if !path.exists() {
        eprintln!("skipping: {} is absent", path.display());
        return;
    }
    let rom = std::fs::read(&path).expect("read Gumshoe");
    let system = System::from_ines(&rom).expect("load Gumshoe");
    assert_eq!(system.cart.mapper(), 99);
    let bank0: Vec<u8> = (0..0x2000u16)
        .map(|offset| system.cart.cpu_read(0x8000 + offset))
        .collect();
    let mut system = system;
    system.cart.write_4016(0x04);
    let bank4: Vec<u8> = (0..0x2000u16)
        .map(|offset| system.cart.cpu_read(0x8000 + offset))
        .collect();
    assert_ne!(bank0, bank4, "Gumshoe outer-bank select at $8000");
}
