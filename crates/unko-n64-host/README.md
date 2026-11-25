# unko-n64-host

Adapter between the `unko-n64` scaffold and the shared `unko-core` interface.

Cartridge parsing is implemented, and the core boots a cartridge's IPL3 on the
VR4300 interpreter with RDRAM/RSP memory and PI/VI stubs. `run_frame` runs RSP
scalar code/DMA, applies RDP fill/triangle commands, presents the VI
framebuffer from RDRAM as RGBA and drains AI DMA into interleaved f32 audio,
resampled from the DAC rate to the requested host sample rate.

A real-cartridge gate is included:

```
UNKO_N64_ROM=/path/to/game.z64 UNKO_N64_FRAMES=60 \
cargo test -p unko-n64-host --lib real_cartridge_validation_from_env -- --ignored --nocapture
```

It boots the supplied image, runs the requested frames and prints FNV-1a
hashes of the video and audio. See `docs/n64-validation.md` for the measured
validation status and remaining approximations.
