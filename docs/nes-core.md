# Using the NES core (`unko-nes`) in your own emulator

`unko-nes` is a standalone, MIT-licensed NES/Famicom core written in Rust. It
does not depend on `unko-core`, `unko-app` or any other Unko crate, so you can
use it as the emulation backend of a completely separate emulator frontend.

```
your frontend ──► unko-nes (this crate)
              └► (optional) unko-nes-api + unko-nes-host ──► unko-core
```

`unko-nes-api` and `unko-nes-host` are only needed if you want Unko's
system-generic `unko_core::Core` interface or the NES-specific extension trait.
The core itself owns no files, windows, audio device or threads.

## Add the dependency

```toml
[dependencies]
unko-nes = { git = "https://github.com/lordra9-svg/unko", branch = "main" }
```

For a lean build without save states, cheats or debugger helpers:

```toml
unko-nes = { git = "https://github.com/lordra9-svg/unko", branch = "main", default-features = false }
```

| Feature | Default | Purpose |
|---|---:|---|
| `save-states` | yes | Whole-machine snapshot serialization. |
| `cheats` | yes | Game Genie and RAM cheat application. |
| `debug` | yes | Debugger peek/poke, disassembly and state inspection. |
| `apu-trace` | no | Per-cycle APU trace used by differential examples. |

## Minimal frame loop

```rust
use unko_nes::System;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rom = std::fs::read("game.nes")?;
    let mut nes = System::from_ines(&rom)?;

    nes.set_sample_rate(48_000.0);
    nes.set_controller(0, 0); // no buttons held

    for _ in 0..60 {
        nes.run_frame();
        let rgba = nes.video();   // 256 × 240 × 4, RGBA8
        let audio = nes.audio();  // mono f32, resampled to sample_rate()
        // present / queue rgba, audio
    }
    Ok(())
}
```

`run_frame()` returns the number of CPU cycles it executed. `video()` is a
borrowed `&[u8]` owned by the PPU (no per-frame copy) and `audio()` is mono
`f32` in `-1.0..=1.0` at the rate last passed to `set_sample_rate` (48 kHz by
default, accessible with `sample_rate()`).

### Other loaders

| Image | Entry point |
|---|---|
| iNES / NES 2.0 `.nes` | `System::from_ines(&bytes)` |
| Famicom Disk System `.fds` | `System::from_fds(&image, &disksys_bios)` |
| NSF / NSFE | `Nsf::from_bytes`, then drive the APU through `Nsf` |
| VS. System | iNES image; input and palette via the VS. setters |

`Cartridge::from_ines` and `Cartridge::from_fds` are available if you want to
inspect the header/mapper before building a `System`.

## Input

`set_controller(port, bits)` takes the `$4016/$4017` shift-register bit order:

| Bit | Button |
|---:|---|
| 0 | A |
| 1 | B |
| 2 | Select |
| 3 | Start |
| 4 | Up |
| 5 | Down |
| 6 | Left |
| 7 | Right |

`unko-nes-api::NesButtons` defines these constants for `InputState`. Ports
1–3 are supported for four-score/multitap use where the mapper/models allow it.

Peripherals are explicit setters on `System`: Zapper
(`set_zapper`), Arkanoid paddle, mouse, Power Pad, Family BASIC keyboard,
Subor keyboard, Famicom 3D glasses, microphone, Datach barcode reader and
Miracle Piano. `set_port_device` selects the device on a port.

## Timing, region and video

- `set_region(Region::{Ntsc, Pal, Dendy})` overrides the header timing.
- `frame_rate()` reports the resulting frame rate (~60.0988 or ~50.007 Hz).
- `set_palette` and `set_palette_table` select the PPU palette.
- The framebuffer is always 256 × 240 RGBA8; PAL/Dendy use the same surface
  with their own timing.

## Audio

- The APU is mono. `set_sample_rate` switches the internal resampler.
- `set_audio_quality` selects Fast/Standard/High resampling kernels.
- `set_audio_filters` models the RF/linear/crisp/warm output filters, and
  `set_channel_gains`/`set_audio_source` expose expansion-audio mixing.
- Expansion audio (VRC6, VRC7, MMC5, Namco 163, Sunsoft 5B) is produced by the
  cartridge implementations and included by default.

## Save states and battery RAM

`save_state()` returns an opaque `Vec<u8>` (`UNKOST21` format, including a ROM
fingerprint). `load_state(&bytes)` restores it and rejects a state from a
different cartridge.

Safety limits (enforced by the core):

- a state larger than 64 MiB is rejected before decoding;
- decoding uses `bincode` with fixint encoding and a 64 MiB limit, so a
  malformed length field cannot request an unbounded allocation.

Treat save states as **untrusted input**: only load files the user created or
files you trust. The format is not a stable cross-version file format.

Battery RAM: `sram()` exposes the cartridge PRG-RAM; persist it next to the
ROM. Datach EEPROM images have dedicated accessors.

## Debugging and determinism

- `debug_state()`, `debug_chr()`, `read_ram`, `vram_peek`,
  `debug_peek`/`debug_poke` and `debug_disassemble` are behind `debug`.
- Optional tracing: `start_apu_trace`/`take_apu_trace`,
  `start_exec_trace`/`take_exec_trace`, `start_expansion_trace`.
- `step_instruction()` advances one CPU instruction when you need finer
  control than `run_frame`.
- The core is deterministic: no wall-clock, OS RNG, threads or I/O. Frame and
  audio output depend only on ROM, inputs, options and the explicit RTC/EEPROM
  setters.

## Validation and accuracy

The core is validated against independent references; the repository keeps the
logs and provenance:

- `nestest` golden log: 8,980/8,980 lines exact;
- blargg CPU/PPU/APU suites, `ppu_vbl_nmi` 12/12 (NMI on the VBL flag dot),
  sprites 19/19, APU/DMC 13/13;
- mapper matrix 65/65 and AccuracyCoin 146/146 non-DRAW;
- a licensed-library run over 7,101 images (see the root `README.md`).

Run the crate's own tests with:

```sh
cargo test -p unko-nes
```

Integration tests that need copyrighted ROMs are `#[ignore]` and read paths
from environment variables (see `crates/unko-nes/tests/` and
`docs/test-rom-inventory.md`).

## Using the Unko `Core` adapter (optional)

If you want the system-generic interface used by Unko itself:

```rust
let core: Box<dyn unko_core::Core> = unko_nes_host::factory();
```

`unko-nes-host` implements `unko_core::Core` and `unko-nes-api::NesCore` and
converts NES-only controls. Your frontend never needs `unko-core` if you call
`unko-nes` directly.

## Licence and provenance

MIT. Parts of the core are adaptations of ares (ISC); see `NOTICE`,
`licenses/ares-LICENSE.txt` and `crates/unko-nes/PROVENANCE.md`.
