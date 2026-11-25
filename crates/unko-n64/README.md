# unko-n64

Nintendo 64 system scaffold. Parses and normalises cartridge images (`.z64`,
`.v64`, `.n64`), implements a VR4300 (MIPS III) integer interpreter, and boots a cartridge's
IPL3 with RDRAM/RSP memory and PI/VI register stubs (PIF HLE). The RSP scaffold
covers DMEM/IMEM, DMA, a scalar interpreter and the vector subset
(`LQV/SQV/LHV/SHV/LBV/SBV`, `VADD/VSUB`, `VMULF/VMULU`, `VMACF/VMACU`,
`VRCP/VRCPL/VRCPH`); the RDP scaffold processes
`SET_COLOR_IMAGE`/`SET_FILL_COLOR`/`FILL_RECTANGLE` into a 16-bit framebuffer
that `run_frame` presents from RDRAM, tracks primitive/environment/combine
state, rasterises unshaded/shaded/textured triangles with the RDP edgewalker
(half-pixel X, 1/4-pixel Y, lmajor and Yl/Ym/Yh subpixel spans), samples
triangles using the interpolated texture coefficient words (SET_TEXTURE_IMAGE/SET_TILE/SET_TILE_SIZE; 4/8/16-bit
RGBA/IA/I texels with clamp/mirror/mask), runs the two-cycle combiner
(`SET_COMBINE_MODE`), and applies the `SET_OTHER_MODES` blender: 1/2-cycle
selectors (pixel/memory/blend/fog colour and alpha), force blend, copy-cycle
bypass, fog, coverage modes with AA subpixel coverage and RGB dithering. It
models the AI register block:
`AI_LEN` DMAs 16-bit stereo pairs from RDRAM into a FIFO that the host turns
into audio. CI palettes load through LOAD_TLUT and 4/8-bit CI texels look up
the TLUT, and texture coordinates are perspective-corrected by the parsed W
coefficients. `SET_MASK_IMAGE`/`SET_PRIMITIVE_DEPTH` drive a Z-buffered depth
pipeline: interpolated 16.16 Z compressed to the RDP's 16-bit depth word with
a 16-bit delta exponent, z compare/update for opaque/transparent/decal/
interpenetrating modes and primitive-depth source selection. Fog-alpha edge
cases, exact depth-blend shifters and audio resampling are next.
