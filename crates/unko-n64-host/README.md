# unko-n64-host

Adapter between the `unko-n64` scaffold and the shared `unko-core` interface.

Cartridge parsing is implemented, and the core boots a cartridge's IPL3 on the
VR4300 interpreter with RDRAM/RSP memory and PI/VI stubs. `run_frame` runs RSP
scalar code/DMA, applies RDP fill/triangle commands, presents the VI
framebuffer from RDRAM as RGBA and drains AI DMA into interleaved f32 audio,
resampled from the DAC rate to the requested host sample rate. Remainder:
remaining GPU coverage.
