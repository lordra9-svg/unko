# unko-n64

Nintendo 64 system scaffold. Parses and normalises cartridge images (`.z64`,
`.v64`, `.n64`), implements a VR4300 (MIPS III) interpreter with a 32-entry
TLB and a single-precision COP1 FPU, and boots a cartridge's IPL3 with RDRAM/RSP memory and PI/VI register stubs (PIF HLE). The RSP scaffold
covers DMEM/IMEM, DMA, a scalar interpreter and the vector subset
(`LQV/SQV/LHV/SHV/LBV/SBV`, `VADD/VSUB`, `VMULF/VMULU`, `VMACF/VMACU`,
`VRCP/VRCPL/VRCPH`); the RDP scaffold processes
`SET_COLOR_IMAGE`/`SET_FILL_COLOR`/`FILL_RECTANGLE` into a 16-bit framebuffer
that `run_frame` presents from RDRAM, tracks primitive/environment/combine
state, rasterises unshaded/shaded/textured triangles with the RDP edgewalker
(half-pixel X, 1/4-pixel Y, lmajor and Yl/Ym/Yh subpixel spans), samples
triangles using the interpolated texture coefficient words (SET_TEXTURE_IMAGE/SET_TILE/SET_TILE_SIZE; 4/8/16-bit
RGBA/IA/I texels with clamp/mirror/mask and per-tile mip/LOD selection:
perspective-corrected s/w derivatives compute the LOD, an explicit level-to-tile mip descriptor
table maps levels to tiles, detail/sharpen modes advance the mip tiles, adjacent levels are blended by the LOD fraction, and that fraction feeds
the combiner LOD source. It runs the two-cycle combiner
(`SET_COMBINE_MODE`), and applies the `SET_OTHER_MODES` blender: 1/2-cycle
selectors (pixel/memory/blend/fog colour and alpha), force blend, copy-cycle
bypass, fog, coverage modes driven by the exact 8-bit subpixel AA mask, RGB
dithering, alpha compare (blend-alpha or dithered threshold) and chroma keying
from the exact 17-bit combiner accumulator, with `SET_KEY_GB`/`SET_KEY_R`
centres/scales and the hardware-noise-table, Convert K4/K5, LOD-fraction and
PrimLODFrac combiner sources. It
models the AI register block:
`AI_LEN` queues double-buffered DMAs of 16-bit stereo pairs from RDRAM; samples
are read on demand as the DAC clock consumes them, buffer completion raises the
AI interrupt at the boundary, `AI_STATUS`/remaining length are live, and the DAC
rate is linearly resampled to the host rate (with underrun decay). CI palettes load through LOAD_TLUT and 4/8-bit CI texels look up
the TLUT, and texture coordinates are perspective-corrected by the parsed W
coefficients. `SET_MASK_IMAGE`/`SET_PRIMITIVE_DEPTH` drive a Z-buffered depth
pipeline: interpolated 16.16 Z compressed to the RDP's 16-bit depth word with
a 16-bit delta exponent, z compare/update for opaque/transparent/decal/
interpenetrating modes (including the interpenetrating coverage curve) and
primitive-depth source selection, with exact
subpixel coverage masks and coverage overflow/wrap handling and memory-alpha depth-delta blender shifters. Precise
exceptions (TLB misses/invalid pages, unaligned loads/stores, SYSCALL/BREAK,
reserved instructions, coprocessor-unusable, integer overflow, delay-slot BD,
maskable interrupts and enabled FPU exceptions) are delivered through CP0
(EPC/Cause/BadVAddr, CE, Status.EXL, refill vs general vectors) and `ERET`
returns to the faulting context, and error-level entries (`ErrorEPC`/`ERL`,
returned through by `ERET`) are driven by the CP0 Count/Compare watchdog,
modelled cache parity faults, an MI-register RCP NMI source, MI interrupt
routing to CPU Int0 (IP2) with mask/Status gating, and reset. The FPU reports Inexact/Underflow/Overflow/
Divide-by-zero/Invalid flags and causes in `fcr31`, reports unimplemented
operations, and honors the FS subnormal flush bit; D-format inexact detection
uses error-free transformations. Double-precision (`D`-format) arithmetic, comparisons, conversions
and 64-bit paired FPR storage are implemented too. Validation status
(including why no real cartridge image is used here) and the remaining
approximation inventory are in `docs/n64-validation.md`.
