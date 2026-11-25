# Using the SNES core (`unko-snes`) in your own emulator

`unko-snes` is a standalone, MIT-licensed Super Nintendo / Super Famicom core.
It has no dependency on `unko-core`, `unko-app` or any other Unko crate, so it
can be the backend of an independent emulator.

```
your frontend ──► unko-snes (this crate)
              └► (optional) unko-snes-api + unko-snes-host ──► unko-core
```

`unko-snes-api` / `unko-snes-host` add Unko's system-generic `Core` adapter;
they are not needed for direct embedders. The core owns no window, audio
device, input backend or file paths.

## Add the dependency

```toml
[dependencies]
unko-snes = { git = "https://github.com/lordra9-svg/unko", branch = "main" }
```

The public surface is documented with `cargo doc -p unko-snes --open`.

## Boot sequence

The SNES needs the 64-byte S-SMP IPL ROM. It is user-supplied firmware and is
never bundled. Install it (and any coprocessor firmware) **before reset**:

```rust
use unko_snes::{Cartridge, Snes};

let rom = std::fs::read("game.sfc")?;
let ipl: [u8; 64] = std::fs::read("ipl.rom")?.try_into().unwrap();

let cart = Cartridge::from_bytes(&rom)?;   // strips a copier header, detects LoROM/HiROM
let mut snes = Snes::new(cart);
snes.set_ipl(ipl);

// Optional firmware for carts that need it:
// snes.set_cx4_firmware(&cx4_rom)?;        // CX4 / Mega Man X2-X3
// snes.set_dsp1_firmware(&dsp_rom)?;       // NEC DSP-1/2/3/4

snes.reset();
```

`Cartridge::from_bytes_lenient` is available for images with an unusual or
missing header; prefer `from_bytes` for normal dumps. `rom.header()` exposes
title, mapping, region, checksum and SRAM size.

## Frame loop

The PPU is dot/scanline driven and raises `frame_ready()` when a frame has been
composited:

```rust
loop {
    while !snes.ppu.frame_ready() {
        snes.step();                       // one S-CPU step (bus access granularity)
    }

    let width  = snes.ppu.frame_width();   // 256, or 512 in hi-res modes
    let height = snes.ppu.frame_height();  // 224/240, doubled when interlaced
    let rgba   = snes.ppu.frame_buffer();  // RGBA8888, width*height*4

    let mut audio = Vec::with_capacity(1_200);
    for (left, right) in snes.apu.drain_samples() { // native 32_040 Hz stereo i16
        audio.push(f32::from(left) / 32_768.0);
        audio.push(f32::from(right) / 32_768.0);
    }
    snes.ppu.acknowledge_frame();

    // present / play
}
```

- `step()` returns the master clocks consumed by that S-CPU step. A frame is
  roughly 357k master clocks on NTSC.
- `run_cycles(max_cycles, max_instructions)` is a convenience loop; it stops
  early on `STP`.
- The frame geometry can change per frame (hi-res, interlace, overscan), so
  query `frame_width()`/`frame_height()` each time.
- The S-DSP produces native 32,040 Hz stereo; resampling is the host's job
  (the Unko adapter reports the native rate and does not resample).

## Region and timing

Timing is taken from the cartridge header's region (NTSC/PAL). There is no
`set_region` override: a cartridge runs in the region it declares. `Snes`
exposes `clock`, PPU line/dot counters and DMA/HDMA state for debuggers.

## Input

Controller state uses the `$4218-$421F` auto-poll bit order
(`unko_snes::io::button`):

| Bit | Button | Bit | Button |
|---:|---|---:|---|
| 15 | B | 7 | A |
| 14 | Y | 6 | X |
| 13 | Select | 5 | L |
| 12 | Start | 4 | R |
| 11 | Up | 9 | Left |
| 10 | Down | 8 | Right |

Use `set_joypad(port, bits)` / `joypad(port)` for ports 0 and 1. The core also
models the auto-poll hardware and the `$4016` serial ports; peripherals beyond
standard pads (mouse, multitap, Super Scope) are not emulated.

## Audio and media

- `apu.drain_samples()` yields `(i16, i16)` stereo pairs from the S-DSP.
- `load_bsx_pack`, `set_msu1_data`, `set_msu1_track` and
  `drain_msu1_samples()` cover Satellaview packs and MSU1 streaming audio.
- `set_sufami_turbo_slot(slot, rom, ram)` populates the two Sufami Turbo slots.

## Save states and SRAM

- `save_state() -> Vec<u8>` writes the versioned `SNES` format
  (`SAVE_STATE_MAGIC`, `SAVE_STATE_VERSION`). The layout is not a stable
  cross-version file format.
- `load_state(&bytes)` validates the magic/version, WRAM/SRAM sizes and nested
  memory lengths. It rejects blobs larger than 64 MiB and decodes `bincode`
  with fixint encoding and a 64 MiB limit, so a malformed length cannot
  allocate without bound.
- `snapshot()`/`restore(&SnesState)` work with the structured state if you want
  to inspect or store it yourself.
- Battery SRAM lives on the cartridge: `snes.cart.sram()` and
  `snes.cart.load_sram(bytes)`. Persist it next to the ROM.

Treat save states as untrusted input: only load files you created or trust.

## Coprocessors and firmware

Implemented: SuperFX (GSU), SA-1, S-DD1, SPC7110 (+EXSPC7110), CX4, NEC
DSP-1/2/3/4, OBC1, MSU1, Sharp S-RTC and the two-slot Sufami Turbo base
cartridge.

Firmware is user-supplied and never bundled: `set_ipl`, `set_cx4_firmware`,
`set_dsp1_firmware`, `load_st018_firmware`/`load_st018_data_rom`,
`load_bsx_pack`. A cart whose firmware is missing runs the base console.

Limited: ST018 and Satellaview/BS-X are modelled only as far as the documented
interfaces above; see `docs/snes-accuracy-divergences.md`.

## Determinism and testing

The core is deterministic: no wall-clock reads in emulation paths, no threads
in core code, and every coprocessor is stepped from explicit master-clock
credit. The `CPU_TRACE_LOG` environment variable can emit a PC trace for
debugging but does not affect emulation.

Validation kept in the repository:

- `cargo test -p unko-snes` (272 unit tests) and the integration tests under
  `crates/unko-snes/tests/` (CPU, SPC700, oracle differential tests);
- pixel gates against a calibrated ares oracle: Super Mario World 600/600
  presents, five reference titles 1200/1200, Mega Man X3 / Star Fox
  10000/10000, Mega Man X2 9996/10000 with torn presents proved;
- 600-frame audio gates: all seven reference titles bit-exact in the default
  configuration (see the root `README.md`);
- 212-entry test-ROM baseline;
- 500-image region/mapper-stratified No-Intro boot sweep (370 LoROM, 125
  HiROM, 5 ExHiROM; 381 NTSC / 119 PAL) at 300 frames per image with 0
  failures, recorded in `docs/snes-library-scan.md`.

Tests needing copyrighted ROMs are `#[ignore]` and read paths from environment
variables (`UNKO_SNES_ROM`, `UNKO_SNES_IPL`, `UNKO_SNES_CX4`,
`UNKO_SNES_FRAMES`, `UNKO_SNES_OUT`). See
`crates/unko-app/src/player.rs` for the gate harness and
`docs/test-rom-inventory.md` for the inventory.

## Using the Unko `Core` adapter (optional)

```rust
let core: Box<dyn unko_core::Core> = unko_snes_host::factory();
```

`unko-snes-host` implements `unko_core::Core` and `unko-snes-api::SnesCore`,
applies firmware via `SnesCore::set_firmware(kind, bytes)` before `load`, and
drains native-rate stereo audio. Direct embedders do not need it.

## Licence and provenance

MIT. ares `sfc/` (ISC) is the primary implementation source; MesenCE (GPL-3.0)
and RustySNES (Apache-2.0) are black-box oracles only. See `NOTICE`,
`licenses/ares-LICENSE.txt`, `crates/unko-snes/PROVENANCE.md` and
`docs/snes-accuracy-divergences.md`.
