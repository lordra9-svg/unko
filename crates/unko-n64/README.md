# unko-n64

Nintendo 64 system scaffold. Parses and normalises cartridge images (`.z64`,
`.v64`, `.n64`), implements a VR4300 (MIPS III) integer interpreter, and boots a cartridge's
IPL3 with RDRAM/RSP memory and PI/VI register stubs (PIF HLE). The RSP scaffold
covers DMEM/IMEM, DMA and a scalar interpreter; the RDP scaffold processes
`SET_COLOR_IMAGE`/`SET_FILL_COLOR`/`FILL_RECTANGLE` into a 16-bit framebuffer
that `run_frame` presents from RDRAM. Textured triangles and audio are next.
