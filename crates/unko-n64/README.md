# unko-n64

Nintendo 64 system scaffold. Parses and normalises cartridge images (`.z64`,
`.v64`, `.n64`), implements a VR4300 (MIPS III) integer interpreter, and boots a cartridge's
IPL3 with RDRAM/RSP memory and PI/VI register stubs (PIF HLE). The RSP scaffold
covers DMEM/IMEM, DMA, a scalar interpreter and the vector subset
(`LQV/SQV/LHV/SHV/LBV/SBV`, `VADD/VSUB`, `VMULF/VMULU`, `VMACF/VMACU`,
`VRCP/VRCPL/VRCPH`); the RDP scaffold processes
`SET_COLOR_IMAGE`/`SET_FILL_COLOR`/`FILL_RECTANGLE` into a 16-bit framebuffer
that `run_frame` presents from RDRAM, tracks primitive/environment/combine
state, rasterises unshaded/shaded triangles (half-space coverage from the edge
coefficients), and models the AI register block: `AI_LEN` DMAs 16-bit stereo
pairs from RDRAM into a FIFO that the host turns into audio. Texturing, full
combiner blending and audio resampling are next.
