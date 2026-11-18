//! S-PPU1/S-PPU2 line-based renderer.
//!
//! The PPU register map, VRAM/CGRAM/OAM access, vblank/NMI timing and
//! background/sprite rendering are implemented here. The renderer produces a
//! 256x224 RGBA8888 frame. Colour math, windows, interlace, hires output and
//! cycle-exact background fetch timing are still approximations; the public
//! base modes 0-7 and sprites are covered.

use crate::rom::Region;

/// Visible frame width.
pub const SCREEN_WIDTH: usize = 256;
/// Visible frame height in non-overscan mode.
///
/// ares `PPU::updateVideoMode`: `vdisp = 225` without overscan, so active
/// display is lines 1-224.
pub const SCREEN_HEIGHT: usize = 224;
/// Output height in overscan mode (`SETINI` bit 2, ares `vdisp = 240`).
///
/// The active lines are 1-239 and row 0 is the top border, matching the ares
/// surface layout (`DAC::scanline` does not add the 8-row vertical offset).
pub const OVERSCAN_HEIGHT: usize = 240;
/// Output width in hi-res modes (5/6, or pseudo-hires `SETINI` bit 3): the DAC
/// emits two columns per pixel clock. `SCREEN_WIDTH` stays the compositing
/// width, matching the reference PPU model.
pub const MAX_SCREEN_WIDTH: usize = 512;
/// Output height in interlace mode: two fields of the active area written on
/// alternate output rows, i.e. 448 lines normally and 480 in overscan.
pub const MAX_SCREEN_HEIGHT: usize = OVERSCAN_HEIGHT * 2;
/// RGBA8888 frame size at the normal (256-wide) resolution.
pub const FRAME_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 4;
/// Backing storage is always maximum capacity so a mode change mid-frame never
/// reallocates; [`Ppu::frame_buffer`] returns a resolution-sized slice of it.
const MAX_FRAME_SIZE: usize = MAX_SCREEN_WIDTH * MAX_SCREEN_HEIGHT * 4;

const DOTS_PER_LINE: u16 = 341;
const LINES_PER_FRAME: u16 = 262;
const VBLANK_LINE: u16 = 225;
/// The S-PPU pair.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ppu {
    vram: Box<[u16]>,
    #[serde(with = "serde_big_array::BigArray")]
    cgram: [u16; 256],
    #[serde(with = "serde_big_array::BigArray")]
    oam: [u8; 0x220],
    framebuffer: Box<[u8]>,
    /// Whether the presented frame is hi-res (latched at frame start).
    frame_hires: bool,
    /// Whether the presented frame is interlaced (latched at frame start).
    frame_interlace: bool,
    /// Whether the presented frame uses the 240-line overscan geometry (latched
    /// at frame start from `SETINI` bit 2, ares `state.overscan`).
    frame_overscan: bool,
    /// Whether this frame's NMI edge has already fired (SETINI can move
    /// `vdisp` mid-frame).
    nmi_fired: bool,
    /// Number of scanlines in the frame being rendered: 262 normally, 263 on
    /// an interlaced even field (ares `PPUcounter::tickScanline`), and the
    /// PAL equivalents 312/313.
    frame_lines: u16,
    /// Dots in the current scanline: 341 normally, 342 on PAL's long
    /// interlaced line (V=311) and 340 on NTSC's short line at V=240.
    line_dots: u16,
    /// Console region: selects the frame geometry and the `$213F` PAL bit.
    region: Region,
    line: u16,
    dot: u16,
    field: bool,
    vblank: bool,
    nmi: bool,
    frame_ready: bool,
    /// Completed frames since power-on (debug).
    pub debug_frame: u64,
    /// $2100 INIDISP
    display_brightness: u8,
    display_disable: bool,
    /// $2101 OBSEL
    obsel: u8,
    /// $2102-$2104 OAM
    oam_base: u16,
    oam_address: u16,
    oam_latch: u8,
    oam_priority: bool,
    first_sprite: u8,
    /// $2105 BGMODE
    bg_mode: u8,
    /// $2106 MOSAIC
    mosaic: u8,
    /// $2107-$210A BGnSC
    bg_sc: [u8; 4],
    /// $210B/$210C BG12NBA/BG34NBA
    bg_nba: [u8; 4],
    /// $210D-$2114 BGnHOFS/VOFS (already assembled 10-bit values)
    bg_h: [u16; 4],
    bg_v: [u16; 4],
    latch_bgofs_ppu1: u8,
    latch_bgofs_ppu2: u8,
    latch_mode7: u8,
    m7_hofs: u16,
    m7_vofs: u16,
    /// $2115 VMAIN
    vmain: u8,
    vram_address: u16,
    vram_read_latch: u16,
    /// $211A M7SEL
    m7sel: u8,
    m7a: i16,
    m7b: i16,
    m7c: i16,
    m7d: i16,
    m7x: i16,
    m7y: i16,
    /// $2121/$2122 CGRAM
    cgram_address: u8,
    cgram_latch: bool,
    cgram_read_latch: u16,
    /// $2123-$2129 windows and main/sub screens
    w12sel: u8,
    w34sel: u8,
    wobjsel: u8,
    wh: [u8; 4],
    wbglog: u8,
    wobjlog: u8,
    tm: u8,
    ts: u8,
    tmw: u8,
    tsw: u8,
    /// $2130-$2132 colour math
    cgwsel: u8,
    cgadsub: u8,
    coldata: u8,
    color_red: u8,
    color_green: u8,
    color_blue: u8,
    /// $2133 SETINI
    setini: u8,
    /// $2137 latched dot/line
    latched_h: u16,
    latched_v: u16,
    /// Separate byte latches for $213C and $213D (ares `latch.hcounter` /
    /// `latch.vcounter`).
    counter_latch_h: bool,
    counter_latch_v: bool,
    /// Whether a `$2137` read latched the counters since the last `$213F`.
    counters_latched: bool,
    /// ares `ppu1.mdr` / `ppu2.mdr`: the PPU-side data registers that back the
    /// read-only register open bus.
    ppu1_mdr: u8,
    ppu2_mdr: u8,
    /// ares `Object::io.rangeOver`: more than 32 sprites on the line.
    range_over: bool,
    /// ares `Object::io.timeOver`: more than 34 object tiles on the line.
    time_over: bool,
    /// ares `latch.oamAddress`: the sprite index the renderer last evaluated;
    /// OAM accesses during active display are redirected here.
    latch_oam_address: u8,
    /// ares `latch.cgramAddress`: the palette index the renderer last looked
    /// up; CGRAM accesses during active display are redirected here.
    latch_cgram_address: u8,
    line_event: bool,
    layer_mask: Option<u8>,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    /// Create a PPU in its power-on state.
    pub fn new() -> Self {
        Self {
            vram: vec![0; 0x8000].into_boxed_slice(),
            cgram: [0; 256],
            oam: [0; 0x220],
            framebuffer: vec![0; MAX_FRAME_SIZE].into_boxed_slice(),
            frame_hires: false,
            frame_interlace: false,
            frame_overscan: false,
            nmi_fired: false,
            frame_lines: LINES_PER_FRAME,
            line_dots: DOTS_PER_LINE,
            region: Region::Ntsc,
            line: 0,
            dot: 0,
            field: false,
            vblank: false,
            nmi: false,
            frame_ready: false,
            debug_frame: 0,
            display_brightness: 0,
            // ares `PPU::power`: the PPU powers up force-blanked.
            display_disable: true,
            obsel: 0,
            oam_base: 0,
            oam_address: 0,
            oam_latch: 0,
            oam_priority: false,
            first_sprite: 0,
            bg_mode: 0,
            mosaic: 0,
            bg_sc: [0; 4],
            bg_nba: [0; 4],
            bg_h: [0; 4],
            bg_v: [0; 4],
            latch_bgofs_ppu1: 0,
            latch_bgofs_ppu2: 0,
            latch_mode7: 0,
            m7_hofs: 0,
            m7_vofs: 0,
            vmain: 0,
            vram_address: 0,
            vram_read_latch: 0,
            m7sel: 0,
            m7a: 0,
            m7b: 0,
            m7c: 0,
            m7d: 0,
            m7x: 0,
            m7y: 0,
            cgram_address: 0,
            cgram_latch: false,
            cgram_read_latch: 0,
            w12sel: 0,
            w34sel: 0,
            wobjsel: 0,
            wh: [0; 4],
            wbglog: 0,
            wobjlog: 0,
            tm: 0,
            ts: 0,
            tmw: 0,
            tsw: 0,
            cgwsel: 0,
            cgadsub: 0,
            coldata: 0,
            color_red: 0,
            color_green: 0,
            color_blue: 0,
            setini: 0,
            latched_h: 0,
            latched_v: 0,
            counter_latch_h: false,
            counter_latch_v: false,
            counters_latched: false,
            ppu1_mdr: 0,
            ppu2_mdr: 0,
            range_over: false,
            time_over: false,
            latch_oam_address: 0,
            latch_cgram_address: 0,
            line_event: false,
            layer_mask: None,
        }
    }

    /// Reset the PPU.
    pub fn reset(&mut self) {
        let region = self.region;
        *self = Self::new();
        self.set_region(region);
    }

    /// Select the console region.
    ///
    /// ares uses the region for the frame length (`vperiod`), the `$213F` PAL
    /// flag and the vertical output offset. The PPU keeps the same 225-line
    /// non-overscan visible window in both regions (`PPU::updateVideoMode`).
    pub fn set_region(&mut self, region: Region) {
        self.region = region;
        self.frame_lines = region.lines_per_frame();
        self.line_dots = DOTS_PER_LINE;
    }

    /// Console region.
    pub const fn region(&self) -> Region {
        self.region
    }

    /// Whether the heap-backed memory regions have their expected sizes.
    ///
    /// Deserialized save states carry variable-length boxed slices; a corrupt
    /// blob must be rejected before indexing panics.
    pub(crate) fn state_lengths_valid(&self) -> bool {
        self.vram.len() == 0x8000 && self.framebuffer.len() == MAX_FRAME_SIZE
    }

    /// Current scanline.
    pub const fn line(&self) -> u16 {
        self.line
    }

    /// Current dot within the scanline.
    pub const fn dot(&self) -> u16 {
        self.dot
    }

    /// Current INIDISP brightness value.
    pub const fn display_brightness(&self) -> u8 {
        self.display_brightness
    }

    /// Read a word from VRAM (debug/test helper).
    pub fn vram_word(&self, address: u16) -> u16 {
        self.vram[usize::from(address & 0x7FFF)]
    }

    /// Render one scanline of the object layer into a fresh buffer.
    /// Diagnostics only; useful for isolating sprite placement from background
    /// composition.
    pub fn debug_sprite_row(&self, y: usize) -> Vec<(usize, u8, u8)> {
        let mut main = [Pixel::default(); SCREEN_WIDTH];
        let mut sub = [Pixel::default(); SCREEN_WIDTH];
        let (_, obj_priority) = self.priority_tables(self.bg_mode & 0x07);
        let _ = self.render_sprites(&mut main, &mut sub, y, &obj_priority);
        main.iter()
            .enumerate()
            .filter(|(_, pixel)| pixel.priority != 0)
            .map(|(x, pixel)| (x, pixel.color, pixel.priority))
            .collect()
    }

    /// Override the main-screen layer mask. Diagnostics only: the mask is not
    /// part of the PPU state and is never serialized.
    pub fn set_layer_mask(&mut self, mask: Option<u8>) {
        self.layer_mask = mask;
    }

    /// Current mosaic register (debug/test helper).
    pub const fn mosaic(&self) -> u8 {
        self.mosaic
    }

    /// Sub-screen enable mask (debug/test helper).
    pub const fn sub_screen(&self) -> u8 {
        self.ts
    }

    /// Mode 7 selection register (debug/test helper).
    pub const fn m7sel(&self) -> u8 {
        self.m7sel
    }

    /// Current OBSEL value (debug/test helper).
    pub const fn obsel(&self) -> u8 {
        self.obsel
    }

    /// Read the object attribute memory (debug/test helper).
    pub fn oam(&self) -> &[u8; 0x220] {
        &self.oam
    }

    /// All 32K VRAM words (debug).
    pub fn vram_words(&self) -> &[u16] {
        &self.vram
    }

    /// Read a CGRAM word (debug/test helper).
    pub fn cgram_word(&self, address: u8) -> u16 {
        self.cgram[usize::from(address)]
    }

    /// Current background mode (debug).
    pub const fn bg_mode(&self) -> u8 {
        self.bg_mode
    }

    /// Debug: `(bg_mode, tm, ts, bg_sc, bg_nba, bg_h, bg_v)`.
    #[allow(clippy::type_complexity)]
    pub const fn debug_regs(&self) -> (u8, u8, u8, [u8; 4], [u8; 4], [u16; 4], [u16; 4]) {
        (
            self.bg_mode,
            self.tm,
            self.ts,
            self.bg_sc,
            self.bg_nba,
            self.bg_h,
            self.bg_v,
        )
    }

    /// Background screen base/mode register (debug).
    pub const fn bg_sc(&self, layer: usize) -> u8 {
        self.bg_sc[layer]
    }

    /// Background character base register (debug).
    pub const fn bg_nba(&self, layer: usize) -> u8 {
        self.bg_nba[layer]
    }

    /// Main-screen enable mask (debug).
    pub const fn main_screen(&self) -> u8 {
        self.tm
    }

    /// Current background scroll (debug).
    pub const fn bg_scroll(&self, layer: usize) -> (u16, u16) {
        (self.bg_h[layer], self.bg_v[layer])
    }

    /// Current VMAIN value (debug).
    pub const fn vmain(&self) -> u8 {
        self.vmain
    }

    /// Current VRAM address (debug).
    pub const fn vram_address(&self) -> u16 {
        self.vram_address
    }

    /// Whether the PPU is in vblank.
    pub const fn in_vblank(&self) -> bool {
        self.vblank
    }

    /// Whether the current dot is in horizontal blanking.
    ///
    /// ares `PPU::readIO` reports `hcounter <= 2 || hcounter >= 1096`; its
    /// `hcounter` is in master clocks and a dot is four of them, so blanking
    /// starts at dot 274.
    pub const fn in_hblank(&self) -> bool {
        self.dot == 0 || self.dot >= 274
    }

    /// Take a pending NMI edge.
    pub fn take_nmi(&mut self) -> bool {
        let pending = self.nmi;
        self.nmi = false;
        pending
    }

    /// Whether a new frame is ready to present.
    pub const fn frame_ready(&self) -> bool {
        self.frame_ready
    }

    /// Output width of the frame currently being composited.
    ///
    /// `SCREEN_WIDTH` normally and `MAX_SCREEN_WIDTH` in hi-res modes, matching
    /// the reference model: the compositing stage always produces 256 above/
    /// below pixel pairs per line and only the output stage doubles.
    pub const fn frame_width(&self) -> usize {
        if self.frame_hires {
            MAX_SCREEN_WIDTH
        } else {
            SCREEN_WIDTH
        }
    }

    /// Whether the current frame is hi-res (512-wide).
    pub const fn frame_hires(&self) -> bool {
        self.frame_hires
    }

    /// Whether the current frame uses the 240-line overscan geometry.
    pub const fn frame_overscan(&self) -> bool {
        self.frame_overscan
    }

    /// Active-display cutoff line.
    ///
    /// ares `PPU::updateVideoMode`: `vdisp = !io.overscan ? 225 : 240`, so
    /// without overscan the active lines are 1-224 and with it 1-239.
    pub const fn vdisp(&self) -> u16 {
        if self.setini & 0x04 != 0 {
            OVERSCAN_HEIGHT as u16
        } else {
            VBLANK_LINE
        }
    }

    /// Last line the current frame's output geometry can draw (1-based).
    pub const fn visible_last(&self) -> u16 {
        if self.frame_overscan {
            OVERSCAN_HEIGHT as u16 - 1
        } else {
            SCREEN_HEIGHT as u16
        }
    }

    /// Output height of the frame currently being composited: 224 normally,
    /// 240 in overscan, doubled when interlaced.
    pub const fn frame_height(&self) -> usize {
        let lines = if self.frame_overscan {
            OVERSCAN_HEIGHT
        } else {
            SCREEN_HEIGHT
        };
        if self.frame_interlace {
            lines * 2
        } else {
            lines
        }
    }

    /// Whether the current frame is interlaced (448- or 480-line output).
    pub const fn frame_interlace(&self) -> bool {
        self.frame_interlace
    }

    /// The RGBA8888 frame buffer for the current frame's geometry.
    pub fn frame_buffer(&self) -> &[u8] {
        &self.framebuffer[..self.frame_width() * self.frame_height() * 4]
    }

    /// Mark the current frame as presented.
    pub fn acknowledge_frame(&mut self) {
        self.frame_ready = false;
    }

    /// Advance one PPU dot.
    pub fn tick(&mut self) {
        self.dot += 1;
        if self.dot < self.line_dots {
            return;
        }
        self.dot = 0;
        self.line += 1;
        self.line_event = true;
        if self.line >= self.frame_lines {
            self.line = 0;
            self.field = !self.field;
            // ares `hires = pseudoHires || bgMode == 5 || bgMode == 6`.
            self.frame_hires = self.setini & 0x08 != 0 || matches!(self.bg_mode & 0x07, 5 | 6);
            // ares `PPUcounter::tickScanline`: the interlaced frame with
            // `field == 0` has one extra line (263 NTSC / 313 PAL).
            self.frame_interlace = self.setini & 0x01 != 0;
            // ares `PPU::main`: `state.overscan = io.overscan` at vcounter 0.
            self.frame_overscan = self.setini & 0x04 != 0;
            self.nmi_fired = false;
            self.frame_lines = if self.frame_interlace && !self.field {
                self.region.lines_per_frame() + 1
            } else {
                self.region.lines_per_frame()
            };
            // The overscan layout starts at output row 1 (progressive) or row 2
            // (interlaced), so the rows above the first active line are the
            // vertical border. Clear them once per frame in case the previous
            // frame used the other geometry.
            if self.frame_overscan {
                let width = self.frame_width();
                let rows = if self.frame_interlace { 2 } else { 1 };
                for row in 0..rows {
                    let start = row * width * 4;
                    self.framebuffer[start..start + width * 4].fill(0);
                }
            }
        }
        // ares `PPUcounter::tickScanline`: NTSC has one short scanline and PAL
        // one long scanline so the average rate tracks the color burst.
        self.line_dots = DOTS_PER_LINE;
        if self.region.is_pal() {
            if self.frame_interlace && self.field && self.line == 311 {
                self.line_dots = DOTS_PER_LINE + 1;
            }
        } else if !self.frame_interlace && self.field && self.line == 240 {
            self.line_dots = DOTS_PER_LINE - 1;
        }
        if self.line == 0 {
            self.vblank = false;
            // ares `Object::frame`: the overflow flags are cleared once per
            // frame and latched by any line that overflows.
            self.range_over = false;
            self.time_over = false;
        }
        // ares `CPU::nmiPoll`: NMI asserts once `vcounter >= vdisp`, where
        // `vdisp` is 225 normally and 240 with overscan (`SETINI` bit 2).
        if self.line >= self.vdisp() && !self.nmi_fired {
            self.nmi_fired = true;
            self.vblank = true;
            self.nmi = true;
        }
        // ares `PPU::main` presents the frame at vcounter 240 in both modes.
        if self.line == 240 {
            self.frame_ready = true;
            self.debug_frame = self.debug_frame.wrapping_add(1);
        }
    }

    /// Take the scanline-change event generated by [`Self::tick`].
    pub fn take_line_event(&mut self) -> Option<u16> {
        if self.line_event {
            self.line_event = false;
            Some(self.line)
        } else {
            None
        }
    }

    /// Render the current visible scanline after HDMA has run.
    ///
    /// Active display is lines 1-224 normally and 1-239 in overscan; line 0 is
    /// always vblank. Background and sprite fetches therefore use the line
    /// number itself, not the framebuffer row.
    pub fn render_current_line(&mut self) {
        if let Some(path) = std::env::var_os("PPU_LINE_LOG") {
            use std::io::Write;
            if (498..=501).contains(&self.debug_frame) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(
                        f,
                        "frame={} line={} bright={} tm={:02X} ts={:02X} cgwsel={:02X} cgadsub={:02X} coldata={:02X} cg0={:04X}",
                        self.debug_frame,
                        self.line,
                        self.display_brightness,
                        self.tm,
                        self.ts,
                        self.cgwsel,
                        self.cgadsub,
                        self.coldata,
                        self.cgram_word(0)
                    );
                }
            }
        }
        if self.line >= 1 && self.line <= self.visible_last() {
            // ares `DAC::scanline`: without overscan the row pointer is
            // `vcounter + 8`, which our cropped output folds into row
            // `vcounter - 1`; with overscan it is `vcounter` (row 0 is the top
            // border). Interlace doubles the row and offsets the odd field by
            // one. Only this field's rows are written; the other field's rows
            // keep the previous field's content, as on hardware.
            let row = if self.frame_interlace {
                let line = if self.frame_overscan {
                    usize::from(self.line)
                } else {
                    usize::from(self.line) - 1
                };
                line * 2 + usize::from(self.field)
            } else if self.frame_overscan {
                usize::from(self.line)
            } else {
                usize::from(self.line - 1)
            };
            self.render_line(usize::from(self.line), row);
        }
    }

    /// Read a PPU register in `$2100-$213F`.
    ///
    /// The read-only registers return a PPU-side data register (`ppu1.mdr` /
    /// `ppu2.mdr`), and `$2137` returns the CPU's open-bus MDR. That is why
    /// this takes `mdr` and why several arms write only their own bits.
    pub fn read(&mut self, address: u16, mdr: u8) -> u8 {
        match address {
            0x2134..=0x2136 => {
                // ares: `n24 result = (i16)io.m7a * (i8)(io.m7b >> 8);`
                let a = i32::from(self.m7a);
                let b = i32::from((self.m7b >> 8) as i8);
                let result = (a * b) as u32;
                let byte = (result >> (8 * (address - 0x2134))) & 0xFF;
                self.ppu1_mdr = byte as u8;
                self.ppu1_mdr
            }
            0x2137 => mdr,
            0x2138 => {
                self.ppu1_mdr = self.read_oam(self.oam_address);
                self.oam_address = (self.oam_address + 1) & 0x3FF;
                self.set_first_sprite();
                self.ppu1_mdr
            }
            0x2139 => {
                self.ppu1_mdr = self.vram_read_latch as u8;
                if self.vmain & 0x80 == 0 {
                    self.vram_read_latch = self.read_vram_word();
                    self.vram_address = self.vram_address.wrapping_add(self.vram_increment());
                }
                self.ppu1_mdr
            }
            0x213A => {
                self.ppu1_mdr = (self.vram_read_latch >> 8) as u8;
                if self.vmain & 0x80 != 0 {
                    self.vram_read_latch = self.read_vram_word();
                    self.vram_address = self.vram_address.wrapping_add(self.vram_increment());
                }
                self.ppu1_mdr
            }
            0x213B => {
                let address = self.cgram_target();
                if self.cgram_latch {
                    self.cgram_latch = false;
                    self.ppu2_mdr =
                        (self.ppu2_mdr & 0x80) | ((self.cgram[address] >> 8) as u8 & 0x7F);
                    self.cgram_address = self.cgram_address.wrapping_add(1);
                } else {
                    self.cgram_latch = true;
                    self.ppu2_mdr = self.cgram[address] as u8;
                }
                self.ppu2_mdr
            }
            0x213C => {
                if self.counter_latch_h {
                    self.ppu2_mdr = (self.ppu2_mdr & !0x01) | ((self.latched_h >> 8) as u8 & 0x01);
                } else {
                    self.ppu2_mdr = self.latched_h as u8;
                }
                self.counter_latch_h = !self.counter_latch_h;
                self.ppu2_mdr
            }
            0x213D => {
                if self.counter_latch_v {
                    self.ppu2_mdr = (self.ppu2_mdr & !0x01) | ((self.latched_v >> 8) as u8 & 0x01);
                } else {
                    self.ppu2_mdr = self.latched_v as u8;
                }
                self.counter_latch_v = !self.counter_latch_v;
                self.ppu2_mdr
            }
            0x213E => {
                // ares sets bits 0-3 (PPU1 version 1); bits 6/7 are the sprite
                // range/time overflow flags latched for this frame.
                self.ppu1_mdr = (self.ppu1_mdr & 0x10)
                    | 0x01
                    | (u8::from(self.range_over) << 6)
                    | (u8::from(self.time_over) << 7);
                self.ppu1_mdr
            }
            0x213F => {
                // ares `latch.hcounter`/`latch.vcounter` reset; bit 4 is the
                // region (0 = NTSC, 1 = PAL) and bit 6 reports whether $2137
                // latched the counters.
                self.counter_latch_h = false;
                self.counter_latch_v = false;
                self.ppu2_mdr = (self.ppu2_mdr & 0x80)
                    | 0x03
                    | (u8::from(self.region.is_pal()) << 4)
                    | (u8::from(self.counters_latched) << 6);
                self.counters_latched = false;
                self.ppu2_mdr | (u8::from(self.field) << 7)
            }
            _ => 0,
        }
    }

    /// Write a PPU register in `$2100-$213F`.
    pub fn write(&mut self, address: u16, data: u8) {
        static PPU_LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *PPU_LOG.get_or_init(|| std::env::var_os("PPU_LOG").is_some());
        if enabled && matches!(address, 0x2100 | 0x2121 | 0x2122 | 0x2130 | 0x2131 | 0x2132) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/ppu.log")
            {
                let _ = writeln!(
                    f,
                    "{} {} {} {:04X} {:02X}",
                    self.debug_frame, self.line, self.dot, address, data
                );
            }
        }
        match address {
            0x2100 => {
                // ares: forcing blank at the start of vblank resets the OAM
                // address to the base and re-latches `firstSprite`.
                if self.display_disable && self.line == self.vdisp() {
                    self.oam_address = self.oam_base << 1;
                    self.set_first_sprite();
                }
                self.display_brightness = data & 0x0F;
                self.display_disable = data & 0x80 != 0;
            }
            0x2101 => self.obsel = data,
            0x2102 => {
                self.oam_base = (self.oam_base & 0x100) | u16::from(data);
                self.oam_address = self.oam_base << 1;
            }
            0x2103 => {
                self.oam_base = (self.oam_base & 0x0FF) | (u16::from(data & 0x01) << 8);
                self.oam_address = self.oam_base << 1;
                self.oam_priority = data & 0x80 != 0;
            }
            0x2104 => {
                let latch_bit = self.oam_address & 1;
                let address = self.oam_address;
                self.oam_address = (self.oam_address + 1) & 0x3FF;
                if latch_bit == 0 {
                    self.oam_latch = data;
                } else {
                    self.write_oam(address & !1, self.oam_latch);
                    self.write_oam((address & !1) + 1, data);
                }
                self.set_first_sprite();
            }
            0x2105 => self.bg_mode = data,
            0x2106 => self.mosaic = data,
            0x2107..=0x210A => self.bg_sc[usize::from(address - 0x2107)] = data,
            0x210B => {
                self.bg_nba[0] = data & 0x0F;
                self.bg_nba[1] = data >> 4;
            }
            0x210C => {
                self.bg_nba[2] = data & 0x0F;
                self.bg_nba[3] = data >> 4;
            }
            0x210D..=0x2114 => {
                let index = usize::from(address - 0x210D);
                let layer = index >> 1;
                if index & 1 == 0 {
                    let value = (u16::from(data) << 8)
                        | u16::from(self.latch_bgofs_ppu1 & !7)
                        | u16::from(self.latch_bgofs_ppu2 & 7);
                    self.bg_h[layer] = value & 0x03FF;
                    self.latch_bgofs_ppu1 = data;
                    self.latch_bgofs_ppu2 = data;
                    if index == 0 {
                        self.m7_hofs = (u16::from(data) << 8) | u16::from(self.latch_mode7);
                        self.latch_mode7 = data;
                    }
                } else {
                    let value = (u16::from(data) << 8) | u16::from(self.latch_bgofs_ppu1);
                    self.bg_v[layer] = value & 0x03FF;
                    self.latch_bgofs_ppu1 = data;
                    if index == 1 {
                        self.m7_vofs = (u16::from(data) << 8) | u16::from(self.latch_mode7);
                        self.latch_mode7 = data;
                    }
                }
            }
            0x2115 => self.vmain = data,
            0x2116 => {
                self.vram_address = (self.vram_address & 0xFF00) | u16::from(data);
                // ares `PPU::writeIO`: writing VMADDL/VMADDH also reloads the
                // VRAM read latch from the new address. Games rely on it - a
                // read-modify-write upload (`STA $2116; LDA $2139; ...`) reads
                // back the high byte it is about to preserve.
                self.vram_read_latch = self.read_vram_word();
            }
            0x2117 => {
                self.vram_address = (self.vram_address & 0x00FF) | (u16::from(data) << 8);
                self.vram_read_latch = self.read_vram_word();
            }
            0x2118 => self.write_vram_byte(data, false),
            0x2119 => self.write_vram_byte(data, true),
            0x211A => self.m7sel = data,
            // ares `PPU::writeIO`: the Mode 7 matrix and centre registers share
            // the single `latch.mode7` byte, so the first write supplies the
            // low byte and the second the high byte. Treating them as a
            // low/high pair left the low byte permanently zero, which collapsed
            // the whole plane to one pixel.
            0x211B => {
                self.m7a = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x211C => {
                self.m7b = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x211D => {
                self.m7c = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x211E => {
                self.m7d = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x211F => {
                self.m7x = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x2120 => {
                self.m7y = ((u16::from(data) << 8) | u16::from(self.latch_mode7)) as i16;
                self.latch_mode7 = data;
            }
            0x2121 => {
                self.cgram_address = data;
                self.cgram_latch = false;
            }
            0x2122 => {
                let address = self.cgram_target();
                if self.cgram_latch {
                    self.cgram_latch = false;
                    self.cgram[address] =
                        (self.cgram[address] & 0x00FF) | (u16::from(data & 0x7F) << 8);
                    self.cgram_address = self.cgram_address.wrapping_add(1);
                } else {
                    self.cgram_latch = true;
                    self.cgram[address] = (self.cgram[address] & 0xFF00) | u16::from(data);
                }
            }
            0x2123 => self.w12sel = data,
            0x2124 => self.w34sel = data,
            0x2125 => self.wobjsel = data,
            0x2126..=0x2129 => self.wh[usize::from(address - 0x2126)] = data,
            0x212A => self.wbglog = data,
            0x212B => self.wobjlog = data,
            0x212C => self.tm = data,
            0x212D => self.ts = data,
            0x212E => self.tmw = data,
            0x212F => self.tsw = data,
            0x2130 => self.cgwsel = data,
            0x2131 => self.cgadsub = data,
            0x2132 => {
                self.coldata = data;
                if data & 0x20 != 0 {
                    self.color_red = data & 0x1F;
                }
                if data & 0x40 != 0 {
                    self.color_green = data & 0x1F;
                }
                if data & 0x80 != 0 {
                    self.color_blue = data & 0x1F;
                }
            }
            0x2133 => self.setini = data,
            0x2137 => {}
            _ => {}
        }
    }

    /// Latch the H/V counters for `$213C`/`$213D` (ares `latchCounters`).
    pub fn latch_counters(&mut self) {
        self.latched_h = self.dot;
        self.latched_v = self.line;
        self.counters_latched = true;
    }

    fn vram_increment(&self) -> u16 {
        match self.vmain & 0x03 {
            0 => 1,
            1 => 32,
            _ => 128,
        }
    }

    fn vram_word_address(&self) -> u16 {
        let address = self.vram_address;
        match (self.vmain >> 2) & 0x03 {
            0 => address,
            1 => ((address >> 8) << 8) | ((address & 0x1F) << 3) | ((address >> 5) & 0x07),
            2 => ((address >> 9) << 9) | ((address & 0x3F) << 3) | ((address >> 6) & 0x07),
            _ => ((address >> 10) << 10) | ((address & 0x7F) << 3) | ((address >> 7) & 0x07),
        }
    }

    fn write_vram_byte(&mut self, data: u8, high: bool) {
        let address = self.vram_word_address() & 0x7FFF;
        // ares `PPU::writeVRAM`: writes during active display are dropped, but
        // the address still advances.
        if self.display_disable || self.line >= self.vdisp() {
            if let Some(path) = std::env::var_os("VRAM_WRITE_LOG") {
                use std::io::Write;
                if self.debug_frame >= 300 {
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(
                            f,
                            "A={:04X} byte={} data={:02X} mode={} inc={} addr={:04X}",
                            address,
                            u8::from(high),
                            data,
                            u8::from(self.vmain & 0x80 != 0),
                            self.vram_increment(),
                            self.vram_address
                        );
                    }
                }
            }
            let word = self.vram[usize::from(address)];
            self.vram[usize::from(address)] = if high {
                (word & 0x00FF) | (u16::from(data) << 8)
            } else {
                (word & 0xFF00) | u16::from(data)
            };
        }
        let increment = self.vram_increment();
        if (high && self.vmain & 0x80 != 0) || (!high && self.vmain & 0x80 == 0) {
            self.vram_address = self.vram_address.wrapping_add(increment);
        }
    }

    /// Mosaic pixel size (`$2106` bits 4-7 plus one).
    fn mosaic_size(&self) -> usize {
        usize::from((self.mosaic >> 4) & 0x0F) + 1
    }

    /// Whether mosaic is enabled for a BG layer.
    fn layer_mosaic(&self, layer: usize) -> bool {
        self.mosaic & (1 << layer) != 0
    }

    /// Window configuration for a layer: BG1-4 = 0-3, OBJ = 4, colour = 5.
    fn window_config(&self, layer: usize) -> (bool, bool, bool, bool, u8) {
        let (selector, mask) = match layer {
            0 => (self.w12sel & 0x0F, self.wbglog & 0x03),
            1 => (self.w12sel >> 4, (self.wbglog >> 2) & 0x03),
            2 => (self.w34sel & 0x0F, (self.wbglog >> 4) & 0x03),
            3 => (self.w34sel >> 4, (self.wbglog >> 6) & 0x03),
            4 => (self.wobjsel & 0x0F, self.wobjlog & 0x03),
            _ => (self.wobjsel >> 4, (self.wobjlog >> 2) & 0x03),
        };
        // W12SEL/W34SEL/WOBJSEL use invert on the even bit and enable on the
        // odd bit (ares `Window` register decode).
        (
            selector & 0x02 != 0,
            selector & 0x01 != 0,
            selector & 0x08 != 0,
            selector & 0x04 != 0,
            mask,
        )
    }

    /// Whether the window masks pixel `x` for `layer`, matching ares
    /// `Window::test`.
    fn window_masks(&self, layer: usize, x: usize) -> bool {
        let (one_enable, one_invert, two_enable, two_invert, mask) = self.window_config(layer);
        // ares `Window::run` samples the right edge before stopping, so the
        // window is inclusive of both ends.
        let inside_one =
            (x >= usize::from(self.wh[0]) && x <= usize::from(self.wh[1])) ^ one_invert;
        let inside_two =
            (x >= usize::from(self.wh[2]) && x <= usize::from(self.wh[3])) ^ two_invert;
        if !one_enable {
            return two_enable && inside_two;
        }
        if !two_enable {
            return inside_one;
        }
        match mask {
            0 => inside_one | inside_two,
            1 => inside_one & inside_two,
            2 => inside_one ^ inside_two,
            _ => !(inside_one | inside_two),
        }
    }

    /// Colour-window gate: `(above, below)`, from `CGWSEL` bits 4-7.
    fn colour_window(&self, x: usize) -> (bool, bool) {
        let test = self.window_masks(5, x);
        let above = match (self.cgwsel >> 6) & 0x03 {
            0 => true,
            1 => test,
            2 => !test,
            _ => false,
        };
        let below = match (self.cgwsel >> 4) & 0x03 {
            0 => true,
            1 => test,
            2 => !test,
            _ => false,
        };
        (above, below)
    }

    /// Recompute the OAM priority-rotation start sprite (`$2103` bit 7).
    fn set_first_sprite(&mut self) {
        self.first_sprite = if self.oam_priority {
            (self.oam_address >> 2) as u8 & 0x7F
        } else {
            0
        };
    }

    /// OAM byte address: the low table is 0-0x1FF and the high table mirrors
    /// every 32 bytes from 0x200, matching ares `OAM::read`/`OAM::write`.
    fn oam_index(address: u16) -> usize {
        if address & 0x200 == 0 {
            usize::from(address & 0x1FF)
        } else {
            0x200 + usize::from(address & 0x1F)
        }
    }

    /// ares `PPU::readVRAM`: VRAM reads return 0 during active display.
    fn read_vram_word(&self) -> u16 {
        if !self.display_disable && self.line < self.vdisp() {
            return 0;
        }
        self.vram[usize::from(self.vram_word_address() & 0x7FFF)]
    }

    /// ares `PPU::readCGRAM`/`writeCGRAM`: during the visible window the access
    /// is redirected to the palette index the renderer last looked up.
    fn cgram_target(&self) -> usize {
        let visible = !self.display_disable
            && self.line > 0
            && self.line < self.vdisp()
            && self.dot >= 22
            && self.dot < 274;
        usize::from(if visible {
            self.latch_cgram_address
        } else {
            self.cgram_address
        })
    }

    /// OAM address during active display: ares `PPU::readOAM`/`writeOAM`
    /// redirect the access to the sprite the renderer last evaluated.
    fn oam_access_index(&self, address: u16) -> usize {
        if !self.display_disable && self.line < self.vdisp() {
            if address & 0x200 == 0 {
                usize::from(self.latch_oam_address) * 4 + usize::from(address & 1)
            } else {
                0x200 + (usize::from(self.latch_oam_address) >> 2)
            }
        } else {
            Self::oam_index(address)
        }
    }

    fn read_oam(&self, address: u16) -> u8 {
        self.oam[self.oam_access_index(address)]
    }

    fn write_oam(&mut self, address: u16, data: u8) {
        let index = self.oam_access_index(address);
        self.oam[index] = data;
    }

    fn render_line(&mut self, y: usize, row: usize) {
        let mut main = [Pixel::default(); SCREEN_WIDTH];
        let mut sub = [Pixel::default(); SCREEN_WIDTH];
        let mode = self.bg_mode & 0x07;
        let (bg_priority, obj_priority) = self.priority_tables(mode);
        let main_mask = self.layer_mask.unwrap_or(self.tm);
        for (layer, priority) in bg_priority.iter().enumerate() {
            let enabled = main_mask & (1 << layer) != 0
                || (self.ts & (1 << layer) != 0 && self.layer_mask.is_none());
            if !enabled {
                continue;
            }
            if mode == 7 && (layer == 0 || (layer == 1 && self.setini & 0x40 != 0)) {
                self.render_mode7(&mut main, &mut sub, y, layer, priority);
            } else if mode <= 6 {
                self.render_background(&mut main, &mut sub, y, layer, mode, priority);
            }
        }
        // ares evaluates the object layer on every line (for OAM latching and
        // the $213E overflow flags) and only draws it when enabled.
        let obj_enabled = main_mask & 0x10 != 0 || self.ts & 0x10 != 0;
        let (range_over, time_over, latch_oam) = if obj_enabled {
            self.render_sprites(&mut main, &mut sub, y, &obj_priority)
        } else {
            let mut scratch_main = [Pixel::default(); SCREEN_WIDTH];
            let mut scratch_sub = [Pixel::default(); SCREEN_WIDTH];
            self.render_sprites(&mut scratch_main, &mut scratch_sub, y, &obj_priority)
        };
        self.range_over |= range_over;
        self.time_over |= time_over;
        self.latch_oam_address = latch_oam;
        self.present_line(row, &main, &sub);
    }

    /// Per-layer priority tables, transcribed from ares `updateVideoMode`.
    fn priority_tables(&self, mode: u8) -> ([[u8; 2]; 4], [u8; 4]) {
        let bg3_priority = self.bg_mode & 0x08 != 0;
        match mode {
            0 => ([[8, 11], [7, 10], [2, 5], [1, 4]], [3, 6, 9, 12]),
            1 if bg3_priority => ([[5, 8], [4, 7], [1, 10], [0, 0]], [2, 3, 6, 9]),
            1 => ([[6, 9], [5, 8], [1, 3], [0, 0]], [2, 4, 7, 10]),
            2..=5 => ([[3, 7], [1, 5], [0, 0], [0, 0]], [2, 4, 6, 8]),
            6 => ([[2, 5], [0, 0], [0, 0], [0, 0]], [1, 3, 4, 6]),
            7 => {
                if self.setini & 0x40 != 0 {
                    ([[3, 3], [1, 5], [0, 0], [0, 0]], [2, 4, 6, 7])
                } else {
                    ([[2, 2], [0, 0], [0, 0], [0, 0]], [1, 3, 4, 5])
                }
            }
            _ => ([[0; 2]; 4], [0; 4]),
        }
    }

    /// Offset-per-tile lookup word from BG3's tilemap (ares
    /// `Background::fetchOffset`): `y_off` is 0 for the horizontal word and 8
    /// for the vertical one, and the horizontal position is snapped to the
    /// tile column.
    fn opt_word(&self, x_col: u32, y_off: u32) -> u16 {
        let layer = 2usize;
        let sc = self.bg_sc[layer];
        let tile_size = u32::from((self.bg_mode >> (4 + layer)) & 1);
        let screen_h = 32u32 << ((sc as u32) & 1);
        let screen_v = 32u32 << (((sc as u32) >> 1) & 1);
        let hsize = (screen_h * 8) << tile_size;
        let vsize = (screen_v * 8) << tile_size;
        let h = (x_col + (u32::from(self.bg_h[layer]) & !7)) & (hsize - 1);
        let v = (y_off + u32::from(self.bg_v[layer])) & (vsize - 1);
        let htile = h >> (3 + tile_size);
        let vtile = v >> (3 + tile_size);
        let mut offset = (htile & 0x1F) | ((vtile & 0x1F) << 5);
        if htile & 0x20 != 0 {
            offset += 0x400;
        }
        if vtile & 0x20 != 0 {
            offset += if screen_h > 32 { 0x800 } else { 0x400 };
        }
        let base = (u32::from(sc) & 0xFC) << 8;
        self.vram[((base + offset) & 0x7FFF) as usize]
    }

    fn render_background(
        &self,
        main: &mut [Pixel; SCREEN_WIDTH],
        sub: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        layer: usize,
        mode: u8,
        priority: &[u8; 2],
    ) {
        let bpp_shift = match (mode, layer) {
            (0, _) => 0,
            (1, 0 | 1) => 1,
            (1, _) => 0,
            (2, 0 | 1) => 1,
            (3, 0) => 2,
            (3, 1) => 1,
            (4, 0) => 2,
            (4, 1) => 0,
            (5, 0) => 1,
            (5, 1) => 0,
            (6, 0) => 1,
            _ => return,
        };
        let sc = self.bg_sc[layer];
        let tile_size = u32::from((self.bg_mode >> (4 + layer)) & 1);
        // ares `Background::hires()`: only modes 5 and 6 fetch at 512 columns.
        let hires = matches!(mode, 5 | 6);
        // ares `Background::fetchNameTable`: `width = 256 << hires()`, so the
        // fetch space (and therefore the map's horizontal extent) doubles in
        // hires. `htiles` becomes 4 there, i.e. a map entry covers 16 columns
        // and supplies two consecutive characters (`character` and
        // `character + 1`).
        let width = if hires { 512 } else { 256 };
        let hsize = (width << tile_size) << (u32::from(sc) & 1);
        let vsize = (width << tile_size) << ((u32::from(sc) >> 1) & 1);
        let htiles = if hires { 4 } else { 3 + tile_size };
        let vtiles = 3 + tile_size;
        let hscroll = u32::from(self.bg_h[layer]);
        let vscroll = u32::from(self.bg_v[layer]);
        let screen_base = (u32::from(sc) & 0xFC) << 8;
        let char_base = (u32::from(self.bg_nba[layer]) & 0x0F) << 12;
        let palette_offset = if mode == 0 { (layer as u16) << 5 } else { 0 };
        let palette_shift = 2u16 << bpp_shift;
        let tile_stride = 8u32 << bpp_shift;
        let main_enable = self.layer_mask.unwrap_or(self.tm) & (1 << layer) != 0;
        let sub_enable = self.ts & (1 << layer) != 0 && self.layer_mask.is_none();
        let hscreen = if u32::from(sc) & 1 != 0 { 0x400 } else { 0 };
        let vscreen = 0x400 + hscreen;

        let mosaic = self.layer_mosaic(layer);
        let mosaic_size = self.mosaic_size() as u32;
        // ares' mosaic repeats the fetched pixel for `size` pixels and reuses
        // the tile row for `size` lines, aligned so line 1 and pixel 0 start a
        // new block.
        let mosaic_y = if mosaic {
            let offset = (y as u32).saturating_sub(1) % mosaic_size;
            (y as u32) - offset
        } else {
            y as u32
        };
        // ares `Background::fetchNameTable`: inside its `hires()` branch an
        // interlaced frame doubles the vertical fetch so each field reads
        // alternate source lines.
        let vbase = if hires && self.frame_interlace {
            // ares `Background::fetchNameTable`:
            // `vpixel = vpixel << 1 | (field() && !mosaic.enable)` - with
            // mosaic on, both fields fetch the same (even) source line.
            (mosaic_y << 1) | u32::from(self.field && !mosaic)
        } else {
            mosaic_y
        };

        // One background pixel: ares `Background::fetchNameTable` +
        // `fetchCharacter`, returning the palette entry and priority bit.
        let fetch = |h: u32, v: u32| -> Option<(u8, u8)> {
            let mut h = h;
            let mut v = v;
            // Offset per tile (ares `Background::fetchNameTable`): modes 2, 4
            // and 6 take their per-column offsets from BG3's tilemap. The
            // valid bit is 13 + layer; mode 4 packs both directions into one
            // word, selected by bit 15.
            if matches!(mode, 2 | 4 | 6) {
                let hlookup = self.opt_word(h, 0);
                let vlookup = self.opt_word(h, 8);
                let valid = 1u16 << (13 + layer as u16);
                if mode == 4 {
                    if hlookup & valid != 0 {
                        if hlookup & 0x8000 == 0 {
                            h = (h + (u32::from(hlookup) & !7) + (hscroll & 7)) & (hsize - 1);
                        } else {
                            v = (mosaic_y + u32::from(vlookup)) & (vsize - 1);
                        }
                    }
                } else {
                    if hlookup & valid != 0 {
                        h = (h + (u32::from(hlookup) & !7) + (hscroll & 7)) & (hsize - 1);
                    }
                    if vlookup & valid != 0 {
                        v = (mosaic_y + u32::from(vlookup)) & (vsize - 1);
                    }
                }
            }
            h &= hsize - 1;
            v &= vsize - 1;
            let htile = h >> htiles;
            let vtile = v >> vtiles;
            let mut offset = (htile & 0x1F) | ((vtile & 0x1F) << 5);
            if htile & 0x20 != 0 {
                offset += hscreen;
            }
            if vtile & 0x20 != 0 {
                offset += vscreen;
            }
            let attributes = self.vram[((screen_base + offset) & 0x7FFF) as usize];
            let mut character = attributes & 0x03FF;
            let hflip = attributes & 0x4000 != 0;
            let vflip = attributes & 0x8000 != 0;
            // ares: a 16-column (hires) or 16-pixel (`tileSize`) map entry
            // supplies two characters; the low half selects the second one.
            if (htiles == 4 || vtiles == 4) && ((h >> 3) & 1 != 0) != hflip {
                character += 1;
            }
            if vtiles == 4 && ((v >> 3) & 1 != 0) != vflip {
                character += 16;
            }
            let mut row = v & 7;
            if vflip {
                row = 7 - row;
            }
            let address = char_base + u32::from(character) * tile_stride + row;
            let bit = if hflip { h & 7 } else { 7 - (h & 7) };
            let word0 = self.vram[(address & 0x7FFF) as usize];
            let mut color = ((word0 >> bit) & 1) | (((word0 >> (8 + bit)) & 1) << 1);
            if bpp_shift >= 1 {
                let word1 = self.vram[((address + 8) & 0x7FFF) as usize];
                color |= (((word1 >> bit) & 1) << 2) | (((word1 >> (8 + bit)) & 1) << 3);
            }
            if bpp_shift >= 2 {
                let word2 = self.vram[((address + 16) & 0x7FFF) as usize];
                let word3 = self.vram[((address + 24) & 0x7FFF) as usize];
                color |= (((word2 >> bit) & 1) << 4)
                    | (((word2 >> (8 + bit)) & 1) << 5)
                    | (((word3 >> bit) & 1) << 6)
                    | (((word3 >> (8 + bit)) & 1) << 7);
            }
            if color == 0 {
                return None;
            }
            let palette_group = (attributes >> 10) & 0x07;
            let palette = palette_offset + (palette_group << palette_shift) + color;
            Some((
                palette as u8,
                priority[usize::from(attributes & 0x2000 != 0)],
            ))
        };

        for x in 0..SCREEN_WIDTH {
            let mosaic_x = if mosaic {
                (x as u32) - (x as u32 % mosaic_size)
            } else {
                x as u32
            };
            // ares `Background::run` runs once per screen and consumes one
            // background pixel per call, so in hires the Below screen shows BG
            // column `2d` and the Above screen column `2d + 1`; the DAC then
            // interleaves below at the even output column and above at the odd.
            let (h_below, h_above) = if hires {
                let base = (mosaic_x << 1).wrapping_add(hscroll << 1);
                // ares `Background::run`: `mosaic.pixel` is only refreshed on the
                // Below call (`x == 0 && (!hires() || screen == Below)`), and the
                // Above call takes the `else if(mosaic.enable) pixel =
                // mosaic.pixel` branch, so with mosaic on both screens show the
                // block-start Below column.
                (base, if mosaic { base } else { base.wrapping_add(1) })
            } else {
                let h = (mosaic_x + hscroll) & (hsize - 1);
                (h, h)
            };
            let v = (vbase + vscroll) & (vsize - 1);
            let (below, above) = if hires {
                (fetch(h_below, v), fetch(h_above, v))
            } else {
                let pixel = fetch(h_below, v);
                (pixel, pixel)
            };
            if let Some((color, layer_priority)) = above {
                if main_enable && !(self.tmw & (1 << layer) != 0 && self.window_masks(layer, x)) {
                    plot(
                        main,
                        x,
                        Pixel {
                            color,
                            priority: layer_priority,
                            layer: layer as u8,
                        },
                    );
                }
            }
            if let Some((color, layer_priority)) = below {
                if sub_enable && !(self.tsw & (1 << layer) != 0 && self.window_masks(layer, x)) {
                    plot(
                        sub,
                        x,
                        Pixel {
                            color,
                            priority: layer_priority,
                            layer: layer as u8,
                        },
                    );
                }
            }
        }
    }

    fn render_mode7(
        &self,
        main: &mut [Pixel; SCREEN_WIDTH],
        sub: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        layer: usize,
        priority: &[u8; 2],
    ) {
        let a = i32::from(self.m7a);
        let b = i32::from(self.m7b);
        let c = i32::from(self.m7c);
        let d = i32::from(self.m7d);
        let hcenter = sign13(self.m7x);
        let vcenter = sign13(self.m7y);
        let hoffset = sign13(self.m7_hofs as i16);
        let voffset = sign13(self.m7_vofs as i16);
        // ares `Background::runMode7`: the mosaic offset is applied to the
        // pre-flip coordinate, and BG2's *vertical* mosaic is gated by BG1's
        // mosaic enable ("BG2 vertical mosaic uses BG1 mosaic enable").
        let mosaic = self.layer_mosaic(layer);
        let mosaic_size = self.mosaic_size() as u32;
        let vertical_mosaic = if layer == 1 {
            self.layer_mosaic(0)
        } else {
            mosaic
        };
        let base_y = y as u32;
        let y = if vertical_mosaic {
            base_y - (base_y.saturating_sub(1) % mosaic_size)
        } else {
            base_y
        };
        let y = if self.m7sel & 0x02 != 0 {
            255 - y as i32
        } else {
            y as i32
        };
        // ares `io.repeatMode7` (M7SEL bits 6-7): 0/1 repeat the map, 2 makes
        // out-of-bounds pixels transparent and 3 fetches character 0.
        let repeat_mode = (self.m7sel >> 6) & 0x03;
        let origin_x = ((a * clip13(hoffset - hcenter)) & !63)
            + ((b * clip13(voffset - vcenter)) & !63)
            + ((b * y) & !63)
            + (hcenter << 8);
        let origin_y = ((c * clip13(hoffset - hcenter)) & !63)
            + ((d * clip13(voffset - vcenter)) & !63)
            + ((d * y) & !63)
            + (vcenter << 8);
        let main_enable = self.layer_mask.unwrap_or(self.tm) & (1 << layer) != 0;
        let sub_enable = self.ts & (1 << layer) != 0 && self.layer_mask.is_none();
        for x in 0..SCREEN_WIDTH {
            let raw_x = x as u32;
            let mosaic_x = if mosaic {
                raw_x - (raw_x % mosaic_size)
            } else {
                raw_x
            };
            let x = if self.m7sel & 0x01 != 0 {
                255 - mosaic_x as i32
            } else {
                mosaic_x as i32
            };
            let pixel_x = (origin_x + a * x) >> 8;
            let pixel_y = (origin_y + c * x) >> 8;
            let out_of_bounds = (pixel_x | pixel_y) & !1023 != 0;
            if out_of_bounds && repeat_mode == 2 {
                continue;
            }
            let tile_x = (pixel_x >> 3) & 0x7F;
            let tile_y = (pixel_y >> 3) & 0x7F;
            let tile = if out_of_bounds && repeat_mode == 3 {
                0
            } else {
                self.vram[((tile_y << 7 | tile_x) & 0x7FFF) as usize] as u8
            };
            let palette_address = ((pixel_y & 7) << 3 | (pixel_x & 7)) as usize;
            let index = usize::from((tile as u16) << 6 | palette_address as u16);
            let mut palette = (self.vram[index & 0x7FFF] >> 8) as u8;
            // ares `Background::runMode7`: BG1 uses `priority[0]`; BG2 selects
            // on palette bit 7 and *then* clears it, and the transparency test
            // happens after that (so `$80` is transparent on BG2).
            let priority = if layer == 0 {
                priority[0]
            } else {
                let bit = palette >> 7;
                palette &= 0x7F;
                priority[usize::from(bit)]
            };
            if palette == 0 {
                continue;
            }
            let pixel = Pixel {
                color: palette,
                priority,
                layer: layer as u8,
            };
            let screen_x = x as usize;
            if main_enable && !(self.tmw & (1 << layer) != 0 && self.window_masks(layer, screen_x))
            {
                plot(main, screen_x, pixel);
            }
            if sub_enable && !(self.tsw & (1 << layer) != 0 && self.window_masks(layer, screen_x)) {
                plot(sub, screen_x, pixel);
            }
        }
    }

    /// Render the object layer for one line.
    ///
    /// Returns the line's `(rangeOver, timeOver, latchOamAddress)` sprite
    /// state; the caller latches it for the frame (ares `Object::evaluate`/
    /// `fetch` and `latch.oamAddress`).
    fn render_sprites(
        &self,
        main: &mut [Pixel; SCREEN_WIDTH],
        sub: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        priority: &[u8; 4],
    ) -> (bool, bool, u8) {
        let base_size = (self.obsel >> 5) & 0x07;
        let mut range_over = false;
        let mut time_over = false;
        let mut latch_oam = 0u8;
        let nameselect = (self.obsel >> 3) & 0x03;
        let tiledata = u32::from(self.obsel & 0x07) << 13;
        let main_enable = self.layer_mask.unwrap_or(self.tm) & 0x10 != 0;
        let sub_enable = self.ts & 0x10 != 0 && self.layer_mask.is_none();
        // Sprites use their own `SETINI` bit (bit 1), not the frame's bit 0.
        let obj_interlace = self.setini & 0x02 != 0;
        // ares `OAM::Object::height`: with object interlace and a base size of
        // 6 or 7, the small size is 16 lines (hardware quirk).
        let sprite_height = |size: u8| -> u16 {
            if size == 0 && obj_interlace && base_size >= 6 {
                16
            } else {
                sprite_size(base_size, size, true)
            }
        };

        // ares `Object::evaluate`: sprites are scanned from `firstSprite`
        // (the `$2103` priority rotation) and the first 32 on-line sprites are
        // kept, however many off-line entries that takes.
        let mut items = [0u8; 32];
        let mut item_count = 0usize;
        for index in 0..128u8 {
            let sprite = self.first_sprite.wrapping_add(index) & 0x7F;
            let base = usize::from(sprite) * 4;
            let high = self.oam[0x200 + usize::from(sprite) / 4];
            let high_bit = (usize::from(sprite) & 3) * 2;
            let size = (high >> (high_bit + 1)) & 1;
            // ares `Object::onScanline`: `height >> interlace` scanlines carry
            // the sprite, each drawing one field's row.
            let height = sprite_height(size) >> u16::from(obj_interlace);
            // Hardware quirk: a sprite's first visible line is OAM Y + 1.
            let first = usize::from(self.oam[base + 1]) + 1;
            if y < first || y >= first + usize::from(height) {
                continue;
            }
            latch_oam = sprite;
            if item_count == 32 {
                // ares `Object::evaluate` stops once more than 32 sprites are
                // on-line and latches `rangeOver` for the frame.
                range_over = true;
                break;
            }
            items[item_count] = sprite;
            item_count += 1;
        }

        // ares `Object::run`: walk the list backwards and overwrite opaque
        // pixels, so the lowest OAM index wins every overlap regardless of the
        // per-sprite priority bits (those only order the layer). The line also
        // draws at most 34 object tiles, in that same order.
        let mut tile_count = 0u32;
        for slot in (0..item_count).rev() {
            let sprite = usize::from(items[slot]);
            let base = sprite * 4;
            let attr = self.oam[base + 3];
            let high = self.oam[0x200 + sprite / 4];
            let high_bit = (sprite & 3) * 2;
            let size = (high >> (high_bit + 1)) & 1;
            let width = sprite_size(base_size, size, false);
            let height = sprite_height(size);
            let x = u16::from(self.oam[base]) | (u16::from((high >> high_bit) & 1) << 8);
            let first = u16::from(self.oam[base + 1]) + 1;
            // ares `Object::fetch`: `y = t.y - sprite.y; if(interlace) y <<= 1`,
            // then the vertical flip, then the odd field takes the odd row.
            let mut row = (y as u16).wrapping_sub(first) & 0xFF;
            if obj_interlace {
                row = row.wrapping_shl(1);
            }
            row = if attr & 0x80 != 0 {
                (height - 1).wrapping_sub(row)
            } else {
                row
            };
            if obj_interlace {
                row = if attr & 0x80 != 0 {
                    row.wrapping_sub(u16::from(self.field))
                } else {
                    row.wrapping_add(u16::from(self.field))
                };
            }
            row &= 0xFF;
            let character = self.oam[base + 2];
            let tile_width = width / 8;
            let layer_priority = priority[usize::from((attr >> 4) & 0x03)];
            for tx in 0..tile_width {
                let sx = x + tx * 8;
                if sx >= SCREEN_WIDTH as u16 {
                    continue;
                }
                // ares `Object::fetch`: `if(t.tileCount++ >= 34) break;`, so
                // the 35th attempted tile latches `timeOver` and is dropped.
                tile_count += 1;
                if tile_count > 34 {
                    time_over = true;
                    break;
                }
                let chrx = (u16::from(character) & 0x0F)
                    + if attr & 0x40 != 0 {
                        tile_width - 1 - tx
                    } else {
                        tx
                    };
                // Object tiles live in a 16-column grid: the row nibble
                // selects a 256-word line and the column nibble a 16-word tile.
                let chry = ((u16::from(character) >> 4) + row / 8) & 0x0F;
                let chrx = chrx & 0x0F;
                let mut address =
                    tiledata + (u32::from(chry) << 8) + (u32::from(chrx) << 4) + u32::from(row & 7);
                if attr & 0x01 != 0 {
                    address += (u32::from(nameselect) + 1) << 12;
                }
                for px in 0..8 {
                    let screen_x = usize::from(sx) + px;
                    if screen_x >= SCREEN_WIDTH {
                        continue;
                    }
                    // Horizontal flip reverses the tile order and reads each
                    // tile from bit 0 upward; it does not mirror the plane bits
                    // a second time.
                    let bit = if attr & 0x40 != 0 { px } else { 7 - px };
                    let word0 = self.vram[(address & 0x7FFF) as usize];
                    let word1 = self.vram[((address + 8) & 0x7FFF) as usize];
                    let color = ((word0 >> bit) & 1)
                        | (((word0 >> (8 + bit)) & 1) << 1)
                        | (((word1 >> bit) & 1) << 2)
                        | (((word1 >> (8 + bit)) & 1) << 3);
                    if color == 0 {
                        continue;
                    }
                    let palette = 128u16 + ((u16::from(attr) >> 1) & 0x07) * 16 + color;
                    let pixel = Pixel {
                        color: palette as u8,
                        priority: layer_priority,
                        layer: 4,
                    };
                    let masked = self.window_masks(4, screen_x);
                    if main_enable && !(self.tmw & 0x10 != 0 && masked) {
                        plot_object(main, screen_x, pixel);
                    }
                    if sub_enable && !(self.tsw & 0x10 != 0 && masked) {
                        plot_object(sub, screen_x, pixel);
                    }
                }
            }
        }
        (range_over, time_over, latch_oam)
    }

    fn present_line(
        &mut self,
        y: usize,
        main: &[Pixel; SCREEN_WIDTH],
        sub: &[Pixel; SCREEN_WIDTH],
    ) {
        // ares models the DAC luma as (1 + l) / 16, with l = 0 mapped to a
        // quarter of that. Brightness 15 is unity; 0 is 1/64, not black.
        let (num, den) = if self.display_disable {
            (0u32, 1u32)
        } else if self.display_brightness == 0 {
            (1, 64)
        } else {
            (u32::from(self.display_brightness) + 1, 16)
        };
        let fixed = self.fixed_color();
        let luma = (num, den);
        let width = self.frame_width();
        let stride = width / SCREEN_WIDTH;
        for x in 0..SCREEN_WIDTH {
            let above = main[x];
            let below = sub[x];
            let above_backdrop = above.priority == 0;
            // ares `DAC::paletteColor` latches the palette index it looked up.
            self.latch_cgram_address = if above_backdrop { 0 } else { above.color };
            let above_layer = if above_backdrop { 5 } else { above.layer };
            let above_color = if above_backdrop {
                self.cgram[0]
            } else {
                self.cgram[usize::from(above.color)]
            };
            let below_transparent = below.priority == 0;
            let below_color = if below_transparent {
                self.cgram[0]
            } else {
                self.cgram[usize::from(below.color)]
            };
            let below_layer = if below_transparent { 5 } else { below.layer };
            // ares `Background::run`: the Above screen is advanced by one dot
            // (`if(screen == Screen::Above) x++`), so in hi-res the Above
            // screen occupies the ODD output column and Below the even one.
            let above_px = self.dac_pixel(
                x,
                above_color,
                above_layer,
                above_backdrop,
                below_color,
                below_transparent,
                luma,
                fixed,
            );
            let below_px = self.dac_pixel(
                x,
                below_color,
                below_layer,
                below_transparent,
                above_color,
                above_backdrop,
                luma,
                fixed,
            );
            let base = y * width + x * stride;
            if self.frame_hires {
                self.framebuffer[base * 4..base * 4 + 4].copy_from_slice(&below_px);
                let odd = base + 1;
                self.framebuffer[odd * 4..odd * 4 + 4].copy_from_slice(&above_px);
            } else {
                self.framebuffer[base * 4..base * 4 + 4].copy_from_slice(&above_px);
            }
        }
    }

    /// One DAC output column: colour math for `color` (the screen drawn to this
    /// column) against `other` (the opposite screen), then the 5-bit to 16-bit
    /// normalization, the DAC luma and the high byte.
    #[allow(clippy::too_many_arguments)]
    fn dac_pixel(
        &self,
        x: usize,
        color: u16,
        layer: u8,
        backdrop: bool,
        other: u16,
        other_transparent: bool,
        luma: (u32, u32),
        fixed: u16,
    ) -> [u8; 4] {
        let (colour_above, colour_below) = self.colour_window(x);
        let enable = (if backdrop {
            self.cgadsub & 0x20 != 0
        } else {
            self.cgadsub & (1 << layer) != 0
        }) && colour_above
            && colour_below;
        let mut color = color;
        if enable {
            let use_sub = self.cgwsel & 0x02 != 0 && !other_transparent;
            let operand = if use_sub { other } else { fixed };
            let halve = self.cgadsub & 0x40 != 0 && use_sub;
            color = blend(color, operand, halve, self.cgadsub & 0x80 != 0);
        }
        let (num, den) = luma;
        let scale = |value: u16| {
            let value = u32::from(value & 0x1F) * 65_535 / 31;
            ((value * num / den) >> 8) as u8
        };
        [scale(color), scale(color >> 5), scale(color >> 10), 0xFF]
    }

    fn fixed_color(&self) -> u16 {
        u16::from(self.color_red)
            | (u16::from(self.color_green) << 5)
            | (u16::from(self.color_blue) << 10)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Pixel {
    color: u8,
    priority: u8,
    layer: u8,
}

/// Plot an object pixel: a later draw replaces an earlier object pixel
/// unconditionally (ares draws the per-line list backwards, so the lowest OAM
/// index wins every overlap), but a background pixel still wins on an equal or
/// higher layer priority.
fn plot_object(buffer: &mut [Pixel; SCREEN_WIDTH], x: usize, pixel: Pixel) {
    let current = buffer[x];
    if current.layer == 4 || pixel.priority > current.priority {
        buffer[x] = pixel;
    }
}

/// Plot a background pixel: higher priority wins, ties keep the earlier layer.
fn plot(buffer: &mut [Pixel; SCREEN_WIDTH], x: usize, pixel: Pixel) {
    if pixel.priority == 0 {
        return;
    }
    if pixel.priority > buffer[x].priority {
        buffer[x] = pixel;
    }
}

/// 15-bit add/subtract blend with optional halving, matching ares `DAC::blend`.
fn blend(x: u16, y: u16, halve: bool, subtract: bool) -> u16 {
    let x = u32::from(x);
    let y = u32::from(y);
    if !subtract {
        if !halve {
            let sum = x + y;
            let carry = (sum - ((x ^ y) & 0x0421)) & 0x8420;
            ((sum - carry) | (carry - (carry >> 5))) as u16
        } else {
            (((x + y) - ((x ^ y) & 0x0421)) >> 1) as u16
        }
    } else {
        let diff = x.wrapping_sub(y).wrapping_add(0x8420);
        let borrow = (diff.wrapping_sub((x ^ y) & 0x8420)) & 0x8420;
        if !halve {
            (diff.wrapping_sub(borrow) & borrow.wrapping_sub(borrow >> 5)) as u16
        } else {
            (((diff.wrapping_sub(borrow) & borrow.wrapping_sub(borrow >> 5)) & 0x7BDE) >> 1) as u16
        }
    }
}

fn sprite_size(base: u8, large: u8, height: bool) -> u16 {
    let widths = [8u16, 8, 8, 16, 16, 32, 16, 16];
    let heights = [8u16, 8, 8, 16, 16, 32, 32, 32];
    let large_widths = [16u16, 32, 64, 32, 64, 64, 32, 32];
    let large_heights = [16u16, 32, 64, 32, 64, 64, 64, 32];
    let index = usize::from(base & 0x07);
    if large != 0 {
        if height {
            large_heights[index]
        } else {
            large_widths[index]
        }
    } else if height {
        heights[index]
    } else {
        widths[index]
    }
}

fn sign13(value: i16) -> i32 {
    if value & 0x1000 != 0 {
        i32::from(value) | !0x1FFF
    } else {
        i32::from(value) & 0x1FFF
    }
}

fn clip13(value: i32) -> i32 {
    if value & 0x2000 != 0 {
        value | !0x3FF
    } else {
        value & 0x3FF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vblank_and_nmi_timing() {
        let mut ppu = Ppu::new();
        for _ in 0..(VBLANK_LINE as u32 * DOTS_PER_LINE as u32) {
            ppu.tick();
        }
        assert!(ppu.in_vblank());
        assert!(ppu.take_nmi());
        assert!(!ppu.take_nmi());
        for _ in 0..(DOTS_PER_LINE as u32 * (LINES_PER_FRAME - VBLANK_LINE) as u32) {
            ppu.tick();
        }
        assert!(!ppu.in_vblank());
    }

    #[test]
    fn cgram_round_trip() {
        let mut ppu = Ppu::new();
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0x1F);
        ppu.write(0x2122, 0x00);
        assert_eq!(ppu.cgram[2], 0x001F);
    }

    #[test]
    fn mosaic_repeats_background_pixels() {
        fn setup() -> Ppu {
            let mut ppu = Ppu::new();
            ppu.write(0x2100, 0x0F); // full brightness
            ppu.write(0x2105, 0x01); // mode 1
            ppu.write(0x2107, 0x04); // BG1 map at $0400
            ppu.write(0x210B, 0x00); // BG1 characters at $0000
            ppu.write(0x2121, 0x01);
            ppu.write(0x2122, 0x1F); // palette colour 1 = red
            ppu.write(0x2122, 0x00);
            ppu.write(0x212C, 0x01); // BG1 on the main screen
                                     // 4bpp tile 0 row 1 (visible line 1): colour 1 at pixels 0 and 4.
            ppu.vram[1] = 0x0088;
            ppu
        }

        let mut ppu = setup();
        ppu.render_line(1, 0);
        let plain: Vec<u8> = ppu.framebuffer[0..8 * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[0])
            .collect();
        assert_eq!(plain, vec![255, 0, 0, 0, 255, 0, 0, 0]);

        ppu.write(0x2106, 0x11); // size 2, BG1 mosaic enabled
        ppu.render_line(1, 0);
        let mosaic: Vec<u8> = ppu.framebuffer[0..8 * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[0])
            .collect();
        assert_eq!(mosaic, vec![255, 255, 0, 0, 255, 255, 0, 0]);
    }

    #[test]
    fn lowest_oam_index_wins_a_sprite_overlap() {
        let mut ppu = Ppu::new();
        ppu.write(0x2101, 0x03); // OBSEL: 8x8 base size, tiles at $6000
        ppu.write(0x212C, 0x10); // objects on the main screen
        ppu.write(0x2102, 0x00);
        ppu.write(0x2103, 0x00);
        // Sprite 0: priority 0, one pixel at (0, line 1).
        for byte in [0x00, 0x00, 0x00, 0x00] {
            ppu.write(0x2104, byte);
        }
        // Sprite 1: priority 3 at the same place, different tile.
        for byte in [0x00, 0x00, 0x01, 0x30] {
            ppu.write(0x2104, byte);
        }
        ppu.vram[0x6000] = 0x0080; // tile 0, row 0: colour 1
        ppu.vram[0x6010] = 0x0080; // tile 1, row 0: colour 1
                                   // ares draws the per-line list backwards, so sprite 0 is drawn last and
                                   // wins even though sprite 1 has the higher priority bits.
        assert_eq!(ppu.debug_sprite_row(1), vec![(0, 129, 3)]);
    }

    /// Set up a Mode 7 plane whose row 0 of tile 0 is colour 2, with an
    /// identity matrix and the centre and scroll offsets at zero.
    fn mode7_plane() -> Ppu {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.bg_mode = 0x07;
        ppu.tm = 0x01;
        ppu.setini = 0x00;
        ppu.m7sel = 0x00;
        ppu.m7a = 0x0100;
        ppu.m7b = 0x0000;
        ppu.m7c = 0x0000;
        ppu.m7d = 0x0100;
        ppu.m7x = 0x0000;
        ppu.m7y = 0x0000;
        ppu.m7_hofs = 0x0000;
        ppu.m7_vofs = 0x0000;
        // Word 0 is both the tilemap entry for cell 0 (tile 0) and character
        // pixel (0,0) of tile 0, whose palette index is the high byte.
        for word in 0..8 {
            ppu.vram[word] = 0x0200;
        }
        ppu.write(0x2121, 0x00);
        ppu.write(0x2122, 0x00); // backdrop: black
        ppu.write(0x2122, 0x00);
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0); // colour 2: green
        ppu.write(0x2122, 0x03);
        ppu
    }

    #[test]
    fn frame_geometry_matches_the_declared_width() {
        let ppu = Ppu::new();
        assert_eq!(ppu.frame_width(), SCREEN_WIDTH);
        assert!(!ppu.frame_hires());
        assert_eq!(ppu.frame_buffer().len(), FRAME_SIZE);
        // Backing storage is hi-res capacity so a future mode change cannot
        // need a reallocation.
        const { assert!(MAX_FRAME_SIZE >= MAX_SCREEN_WIDTH * SCREEN_HEIGHT * 4) };
    }

    #[test]
    fn hires_map_entries_cover_sixteen_columns() {
        // ares `Background::fetchNameTable`: in hires `width = 256 << hires()`
        // and `htiles = 4`, so one map entry covers 16 columns and supplies two
        // consecutive characters (`character` and `character + 1`). The Below
        // screen samples BG column `2d` and the Above screen column `2d + 1`.
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.bg_mode = 0x05; // mode 5 (hires)
        ppu.frame_hires = true;
        ppu.tm = 0x01;
        ppu.ts = 0x01;
        ppu.write(0x2107, 0x04); // BG1 map at $0400, 32x32
        ppu.write(0x210B, 0x00); // BG1 characters at $0000
        ppu.write(0x2121, 0x01);
        ppu.write(0x2122, 0x1F); // colour 1: red
        ppu.write(0x2122, 0x00);
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0); // colour 2: green
        ppu.write(0x2122, 0x03);
        // Map entry 0: character 0, palette group 0, priority 0.
        ppu.vram[0x400] = 0x0000;
        // 4bpp tile 0 (colour 1 in pixels 0-1) and tile 1 (colour 2 in pixel
        // 0), source row 1.
        ppu.vram[1] = 0x00C0;
        ppu.vram[16 + 1] = 0x8000;
        ppu.line = 1;
        ppu.render_current_line();

        let stride = MAX_SCREEN_WIDTH * 4;
        // Output column 0 = Below = BG column 0 = tile 0 (red).
        assert_eq!(&ppu.framebuffer[0..4], &[255, 0, 0, 255]);
        // Output column 8 = Below = BG column 8 = character + 1 = tile 1.
        assert_eq!(&ppu.framebuffer[8 * 4..8 * 4 + 4], &[0, 255, 0, 255]);
        // Output column 1 = Above = BG column 1 = still tile 0.
        assert_eq!(&ppu.framebuffer[4..8], &[255, 0, 0, 255]);
        // Row 1 belongs to the other field and is untouched.
        assert_eq!(&ppu.framebuffer[stride..stride + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn hires_mosaic_above_screen_reads_the_below_latch() {
        // ares `Background::run`: in hires `mosaic.pixel` is only refreshed on
        // the Below call (`x == 0 && (!hires() || screen == Below)`, or when the
        // counter expires), and the Above call takes the
        // `else if(mosaic.enable) pixel = mosaic.pixel` branch. With mosaic on,
        // both screens therefore show the block-start *Below* column.
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.bg_mode = 0x05; // mode 5 (hires)
        ppu.frame_hires = true;
        ppu.tm = 0x01;
        ppu.ts = 0x01;
        ppu.write(0x2107, 0x04); // BG1 map at $0400
        ppu.write(0x210B, 0x00); // BG1 characters at $0000
        ppu.write(0x2106, 0x11); // mosaic size 2, BG1 mosaic enabled
        ppu.write(0x2121, 0x01);
        ppu.write(0x2122, 0x1F); // colour 1: red
        ppu.write(0x2122, 0x00);
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0); // colour 2: green
        ppu.write(0x2122, 0x03);
        // Tile 0 row 1: colour 1 in BG column 0, colour 2 in column 4 (one hires
        // dot covers two background columns); the columns between are
        // transparent.
        ppu.vram[1] = 0x0880;
        ppu.line = 1;
        ppu.render_current_line();

        // Mosaic block 1 (dots 0-1) shows BG column 0 on both screens; block 2
        // (dots 2-3) shows column 4 on both. Without the latch the Above
        // columns would sample columns 1 and 5, which are transparent.
        assert_eq!(&ppu.framebuffer[0..4], &[255, 0, 0, 255], "below column 0");
        assert_eq!(&ppu.framebuffer[4..8], &[255, 0, 0, 255], "above, latch");
        assert_eq!(
            &ppu.framebuffer[2 * 4..2 * 4 + 4],
            &[255, 0, 0, 255],
            "second dot of block 1"
        );
        assert_eq!(
            &ppu.framebuffer[4 * 4..4 * 4 + 4],
            &[0, 255, 0, 255],
            "below column 4"
        );
        assert_eq!(
            &ppu.framebuffer[5 * 4..5 * 4 + 4],
            &[0, 255, 0, 255],
            "above, latch (block 2)"
        );
    }

    #[test]
    fn interlace_latches_from_setini_and_doubles_the_output_height() {
        // ares `PPU::main`: `state.interlace = io.interlace` at frame start, and
        // the DAC then uses `yScale = 2`, so an interlaced frame is 448 lines.
        let mut ppu = Ppu::new();
        ppu.write(0x2133, 0x01); // SETINI: interlace
        for _ in 0..2 * (LINES_PER_FRAME as u32 + 2) * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert!(ppu.frame_interlace());
        assert_eq!(ppu.frame_height(), SCREEN_HEIGHT * 2);
        assert_eq!(
            ppu.frame_buffer().len(),
            SCREEN_WIDTH * SCREEN_HEIGHT * 2 * 4
        );

        ppu.write(0x2133, 0x00);
        for _ in 0..2 * (LINES_PER_FRAME as u32 + 2) * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert!(!ppu.frame_interlace());
        assert_eq!(ppu.frame_height(), SCREEN_HEIGHT);
    }

    #[test]
    fn overscan_extends_the_visible_area_to_240_lines() {
        // ares `PPU::updateVideoMode`: `vdisp = !io.overscan ? 225 : 240`, and
        // `PPU::main` latches `state.overscan` at vcounter 0.
        let mut ppu = Ppu::new();
        assert_eq!(ppu.vdisp(), VBLANK_LINE);
        assert_eq!(ppu.frame_height(), SCREEN_HEIGHT);

        ppu.write(0x2133, 0x04); // SETINI: overscan
        assert_eq!(ppu.vdisp(), OVERSCAN_HEIGHT as u16);
        // Tick exactly one non-interlaced frame so the geometry latches.
        for _ in 0..LINES_PER_FRAME as u32 * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert_eq!(ppu.line(), 0);
        assert!(ppu.frame_overscan());
        assert_eq!(ppu.frame_height(), OVERSCAN_HEIGHT);
        assert_eq!(ppu.visible_last(), OVERSCAN_HEIGHT as u16 - 1);

        // Back to 224 lines with no overscan.
        ppu.write(0x2133, 0x00);
        for _ in 0..LINES_PER_FRAME as u32 * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert!(!ppu.frame_overscan());
        assert_eq!(ppu.frame_height(), SCREEN_HEIGHT);
    }

    #[test]
    fn overscan_moves_vblank_and_nmi_to_line_240() {
        let mut ppu = Ppu::new();
        ppu.write(0x2133, 0x04); // SETINI: overscan
                                 // Skip to line 224 (still active in overscan).
        for _ in 0..224 * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert_eq!(ppu.line(), 224);
        assert!(!ppu.vblank);
        // Beneath the old 225-line cutoff but still active.
        for _ in 0..DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert_eq!(ppu.line(), 225);
        assert!(!ppu.vblank);
        // Reaching 240 latches vblank and the NMI edge.
        for _ in 0..15 * DOTS_PER_LINE as u32 {
            ppu.tick();
        }
        assert_eq!(ppu.line(), 240);
        assert!(ppu.vblank);
        assert!(ppu.take_nmi());
    }

    #[test]
    fn interlace_writes_both_field_rows() {
        // ares `DAC::scanline`: the odd field is offset by one output row, so
        // the two fields interleave into 448 lines. Each field writes only its
        // own rows; the other field's rows keep the previous field's content.
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.write(0x2105, 0x01); // mode 1
        ppu.write(0x2107, 0x04); // BG1 map at $0400
        ppu.write(0x2121, 0x01);
        ppu.write(0x2122, 0x1F); // colour 1: red
        ppu.write(0x2122, 0x00);
        ppu.write(0x212C, 0x01); // BG1 on the main screen
        for row in 0..8 {
            ppu.vram[row] = 0x0080; // tile 0, pixel 0 of every row: colour 1
        }
        ppu.frame_interlace = true;
        ppu.frame_hires = false;
        ppu.line = 1;
        let stride = SCREEN_WIDTH * 4;

        ppu.field = false;
        ppu.render_current_line();
        assert_eq!(
            &ppu.framebuffer[0..4],
            &[255, 0, 0, 255],
            "even field row 0"
        );
        // The other field's row has not been rendered yet (the framebuffer is
        // untouched until a field writes it).
        assert_eq!(&ppu.framebuffer[stride..stride + 4], &[0, 0, 0, 0]);

        ppu.field = true;
        ppu.render_current_line();
        assert_eq!(
            &ppu.framebuffer[0..4],
            &[255, 0, 0, 255],
            "the even field's row is retained"
        );
        assert_eq!(
            &ppu.framebuffer[stride..stride + 4],
            &[255, 0, 0, 255],
            "odd field row 1"
        );
    }

    #[test]
    fn hires_interlace_doubles_the_vertical_fetch() {
        // ares `Background::fetchNameTable`: inside the hires branch an
        // interlaced frame fetches `vpixel << 1 | field`, so each field takes
        // alternate source lines.
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.bg_mode = 0x05; // mode 5 (hires)
        ppu.frame_hires = true;
        ppu.frame_interlace = true;
        ppu.tm = 0x01;
        ppu.write(0x2107, 0x04); // BG1 map at $0400, 32x32
        ppu.write(0x210B, 0x00); // BG1 characters at $0000
        ppu.write(0x2121, 0x01);
        ppu.write(0x2122, 0x1F); // colour 1: red
        ppu.write(0x2122, 0x00);
        // 4bpp tile 0: source rows 2 and 3 put colour 1 in pixels 0 and 1.
        ppu.vram[2] = 0x00C0;
        ppu.vram[3] = 0x00C0;
        ppu.line = 1;

        // Field 0 fetches source row 2, field 1 fetches row 3. Both land at
        // output row 2 of their own field but in different output rows.
        ppu.field = false;
        ppu.render_current_line();
        // In hires the DAC writes Below at the even column and Above at the
        // odd one; the test enables only the main screen, so check column 1.
        let even = 4;
        assert_eq!(&ppu.framebuffer[even..even + 4], &[255, 0, 0, 255]);

        // Clear row 2: field 0 now falls on an empty row, field 1 still hits 3.
        ppu.vram[2] = 0;
        ppu.field = false;
        ppu.render_current_line();
        let even = 4;
        assert_eq!(&ppu.framebuffer[even..even + 4], &[0, 0, 0, 255]);
        ppu.field = true;
        ppu.render_current_line();
        let odd = (MAX_SCREEN_WIDTH + 1) * 4;
        assert_eq!(&ppu.framebuffer[odd..odd + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn offset_per_tile_shifts_a_bg_column_from_bg3() {
        // ares `Background::fetchNameTable`: in modes 2/4/6 each 8-pixel column
        // of BG1/BG2 takes its horizontal and vertical offsets from BG3's
        // tilemap, gated by bit `13 + layer`. Column 1's horizontal word is set
        // to `$2008` (valid | 8), which moves that column's fetch from tile 1
        // to tile 2.
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.write(0x2105, 0x02); // mode 2
        ppu.write(0x2107, 0x04); // BG1 map at $0400
        ppu.write(0x2109, 0x08); // BG3 map at $0800 (offset-per-tile source)
        ppu.write(0x210B, 0x00); // BG1 characters at $0000
        ppu.write(0x212C, 0x01); // BG1 on the main screen
                                 // BG1 map: column 0 -> tile 0 (colour 1), column 1 -> tile 1 (colour
                                 // 2), column 2 -> tile 2 (colour 1).
        ppu.vram[0x400] = 0x0000;
        ppu.vram[0x401] = 0x0001;
        ppu.vram[0x402] = 0x0002;
        // 4bpp characters: plane 0 only -> colour 1, plane 1 only -> colour 2.
        for row in 0..8 {
            ppu.vram[row] = 0x00FF;
            ppu.vram[16 + row] = 0xFF00;
            ppu.vram[32 + row] = 0x00FF;
        }
        ppu.write(0x2121, 0x01);
        ppu.write(0x2122, 0x1F); // colour 1: red
        ppu.write(0x2122, 0x00);
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0); // colour 2: green
        ppu.write(0x2122, 0x03);
        // BG3 column 1 horizontal word: valid for BG1 (bit 13) with offset 8.
        ppu.vram[0x801] = 0x2008;

        ppu.render_line(0, 0);
        assert_eq!(&ppu.framebuffer[0..4], &[255, 0, 0, 255], "column 0");
        assert_eq!(
            &ppu.framebuffer[8 * 4..8 * 4 + 4],
            &[255, 0, 0, 255],
            "column 1 shifted onto tile 2"
        );

        // Clearing the lookup word disables the shift: column 1 shows tile 1.
        ppu.vram[0x801] = 0x0000;
        ppu.render_line(0, 0);
        assert_eq!(&ppu.framebuffer[8 * 4..8 * 4 + 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn mode7_matrix_registers_use_the_shared_latch() {
        // ares `PPU::writeIO` $211B-$2120: `io.m7a = data << 8 | latch.mode7`
        // with `latch.mode7 = data`, i.e. the first write supplies the low byte
        // and the second the high byte. The same latch is shared with $210D/$210E.
        let mut ppu = Ppu::new();
        ppu.write(0x211B, 0x34);
        ppu.write(0x211B, 0x12);
        assert_eq!(ppu.m7a, 0x1234);
        ppu.write(0x211C, 0x78);
        ppu.write(0x211C, 0x56);
        assert_eq!(ppu.m7b, 0x5678);
        ppu.write(0x211D, 0xBC);
        ppu.write(0x211D, 0x9A);
        assert_eq!(ppu.m7c, 0x9ABC_u16 as i16);
        ppu.write(0x211E, 0xF0);
        ppu.write(0x211E, 0xDE);
        assert_eq!(ppu.m7d, 0xDEF0_u16 as i16);
        ppu.write(0x211F, 0x00);
        ppu.write(0x211F, 0x01);
        assert_eq!(ppu.m7x, 0x0100);
        ppu.write(0x2120, 0x00);
        ppu.write(0x2120, 0x01);
        assert_eq!(ppu.m7y, 0x0100);
    }

    #[test]
    fn mode7_repeat_mode_2_is_transparent_and_3_uses_tile_zero() {
        // `M7SEL` bits 6-7 (ares `io.repeatMode7`): 2 makes out-of-bounds
        // pixels transparent, 3 fetches character 0 instead of repeating.
        // M7A = 8.0 pushes x = 200 to pixel 1600, well outside the 1024x1024
        // map.
        let mut transparent = mode7_plane();
        transparent.m7a = 0x0800;
        transparent.m7sel = 0x80;
        transparent.render_line(0, 0);
        assert_eq!(
            &transparent.framebuffer[200 * 4..200 * 4 + 4],
            &[0, 0, 0, 255]
        );

        let mut tile_zero = mode7_plane();
        tile_zero.m7a = 0x0800;
        tile_zero.m7sel = 0xC0;
        tile_zero.render_line(0, 0);
        assert_eq!(
            &tile_zero.framebuffer[200 * 4..200 * 4 + 4],
            &[0, 255, 0, 255]
        );
    }

    #[test]
    fn mode7_extbg_moves_bit7_pixels_to_the_second_plane() {
        // ares `runMode7`: BG2 selects its priority from palette bit 7 and then
        // clears it, so a pixel with palette `$82` is drawn by BG1 at priority
        // 3 with colour `$82`, then by BG2 at priority 5 with colour `$02`.
        let mut plain = mode7_plane();
        for word in 0..8 {
            plain.vram[word] = 0x8200;
        }
        plain.write(0x2121, 0x82);
        plain.write(0x2122, 0x1F); // colour $82: red
        plain.write(0x2122, 0x00);
        plain.render_line(0, 0);
        assert_eq!(&plain.framebuffer[0..4], &[255, 0, 0, 255]);

        let mut extbg = mode7_plane();
        for word in 0..8 {
            extbg.vram[word] = 0x8200;
        }
        extbg.write(0x2121, 0x82);
        extbg.write(0x2122, 0x1F); // colour $82: red
        extbg.write(0x2122, 0x00);
        extbg.setini = 0x40; // EXTBG
        extbg.tm = 0x03; // BG1 + BG2 on the main screen
        extbg.render_line(0, 0);
        assert_eq!(&extbg.framebuffer[0..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn window_masks_follow_the_selector_and_logic() {
        let mut ppu = Ppu::new();
        ppu.write(0x2126, 10); // WH0: window 1 left
        ppu.write(0x2127, 20); // WH1: window 1 right
                               // BG1 window 1 enabled, not inverted (enable is bit 1).
        ppu.write(0x2123, 0x02);
        assert!(!ppu.window_masks(0, 9));
        assert!(ppu.window_masks(0, 10));
        assert!(ppu.window_masks(0, 19));
        // The right edge is inclusive, matching ares' sampling order.
        assert!(ppu.window_masks(0, 20));
        assert!(!ppu.window_masks(0, 21));

        // Invert BG1's window 1.
        ppu.write(0x2123, 0x03);
        assert!(ppu.window_masks(0, 9));
        assert!(!ppu.window_masks(0, 10));

        // Both windows with AND logic: only the overlap masks.
        ppu.write(0x2128, 10); // WH2
        ppu.write(0x2129, 15); // WH3
        ppu.write(0x212A, 0x01); // BG1 logic = AND
        ppu.write(0x2123, 0x0A); // BG1 window 1 + window 2
        assert!(!ppu.window_masks(0, 17));
        assert!(ppu.window_masks(0, 12));
    }

    #[test]
    fn colour_window_gates_colour_math() {
        let mut ppu = Ppu::new();
        ppu.write(0x2126, 0);
        ppu.write(0x2127, 32);
        // Colour window uses window 1 (enable is bit 5).
        ppu.write(0x2125, 0x20);
        // CGWSEL above mask = 1 (inside), below mask = 0 (always).
        ppu.write(0x2130, 0x40);
        assert_eq!(ppu.colour_window(16), (true, true));
        assert_eq!(ppu.colour_window(40), (false, true));
    }

    #[test]
    fn oam_round_trip() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x80); // force blank: OAM is otherwise inaccessible
        ppu.write(0x2102, 0x00);
        ppu.write(0x2103, 0x00);
        ppu.write(0x2104, 0x12);
        ppu.write(0x2104, 0x34);
        ppu.write(0x2104, 0x56);
        ppu.write(0x2104, 0x78);
        assert_eq!(ppu.oam[0], 0x12);
        assert_eq!(ppu.oam[1], 0x34);
        assert_eq!(ppu.oam[2], 0x56);
        assert_eq!(ppu.oam[3], 0x78);
    }

    #[test]
    fn sprite_rows_use_the_object_tile_grid() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x80); // force blank for OAM access
        ppu.write(0x2101, 0x03); // OBSEL: base size 0, tiledata $6000 words
        ppu.write(0x212C, 0x10); // objects on the main screen
        ppu.write(0x2102, 0x00);
        ppu.write(0x2103, 0x00);
        // Sprite 0: x=0, y=0, tile 0, no flips.
        for _ in 0..4 {
            ppu.write(0x2104, 0x00);
        }
        // High table: size bit set -> 16x16.
        ppu.write(0x2103, 0x01);
        ppu.write(0x2104, 0x02);
        ppu.write(0x2104, 0x00);
        // Tile 0 and tile 16 (the second 16x16 row) each have one pixel in
        // their first column.
        ppu.vram[0x6000] = 0x0080;
        ppu.vram[0x6100] = 0x0080;

        // A sprite starts one line below its OAM Y, and a 16x16 sprite's lower
        // half reads tile 16, not tile 1. Mode 0 object priority for attribute
        // priority 0 is 3.
        assert!(ppu.debug_sprite_row(0).is_empty());
        assert_eq!(ppu.debug_sprite_row(1), vec![(0, 129, 3)]);
        assert_eq!(ppu.debug_sprite_row(9), vec![(0, 129, 3)]);
    }

    #[test]
    fn oam_base_is_a_word_address() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x80); // force blank for OAM access
                                 // Word address 2 -> byte address 4.
        ppu.write(0x2102, 0x02);
        ppu.write(0x2104, 0xAA);
        ppu.write(0x2104, 0xBB);
        assert_eq!(ppu.oam[4], 0xAA);
        assert_eq!(ppu.oam[5], 0xBB);

        // High table: word address 0x100 -> byte address 0x200.
        ppu.write(0x2102, 0x00);
        ppu.write(0x2103, 0x01);
        ppu.write(0x2104, 0x11);
        ppu.write(0x2104, 0x22);
        assert_eq!(ppu.oam[0x200], 0x11);
        assert_eq!(ppu.oam[0x201], 0x22);

        // Word address 0x118 -> byte 0x230, which mirrors to 0x210.
        ppu.write(0x2102, 0x18);
        ppu.write(0x2104, 0x33);
        ppu.write(0x2104, 0x44);
        assert_eq!(ppu.oam[0x210], 0x33);
        assert_eq!(ppu.oam[0x211], 0x44);
    }

    #[test]
    fn mode1_background_renders_a_pixel() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // full brightness
        ppu.write(0x2105, 0x01); // mode 1
        ppu.write(0x2107, 0x04); // BG1SC base $0400, 32x32
        ppu.write(0x210B, 0x00); // BG1 character base 0
        ppu.write(0x2121, 0x00);
        ppu.write(0x2122, 0x1F); // backdrop/color 0 red
        ppu.write(0x2122, 0x00);
        // Palette color 2 = green.
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0);
        ppu.write(0x2122, 0x03);
        // Tile 0 row 0 has color 1 in the left-most pixel.
        ppu.vram[0] = 0x8000;
        ppu.write(0x212C, 0x01);
        ppu.render_line(0, 0);
        assert_eq!(&ppu.framebuffer[0..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn oam_access_is_redirected_during_active_display() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F); // display on (leaves force blank)
        ppu.line = 100; // active display
        ppu.latch_oam_address = 1; // the renderer last evaluated sprite 1
        ppu.write(0x2102, 0);
        ppu.write(0x2103, 0);
        for byte in [0x11, 0x22, 0x33, 0x44] {
            ppu.write(0x2104, byte);
        }
        // ares redirects to `latch.oamAddress << 2 | address & 1`, so only the
        // first word of sprite 1 is touched.
        assert_eq!(ppu.oam[4..8], [0x33, 0x44, 0, 0]);
        assert_eq!(ppu.read(0x2138, 0), 0x33);
        // Force blank addresses OAM normally again.
        ppu.write(0x2100, 0x80);
        ppu.write(0x2102, 0);
        ppu.write(0x2103, 0);
        for byte in [0x11, 0x22, 0x33, 0x44] {
            ppu.write(0x2104, byte);
        }
        assert_eq!(ppu.oam[..4], [0x11, 0x22, 0x33, 0x44]);
        ppu.write(0x2102, 0);
        ppu.write(0x2103, 0);
        assert_eq!(ppu.read(0x2138, 0), 0x11);
    }

    #[test]
    fn vram_is_inaccessible_during_active_display() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F);
        ppu.write(0x2115, 0x80); // increment after the high byte
        ppu.line = 100;
        ppu.write(0x2116, 0x00);
        ppu.write(0x2117, 0x00);
        ppu.write(0x2118, 0x12);
        ppu.write(0x2119, 0x34);
        assert_ne!(ppu.vram[0], 0x3412, "VRAM writes must be dropped");
        // The address still advances after the high byte.
        assert_eq!(ppu.vram_address, 1);
        // Reads return 0 during active display.
        ppu.vram[0] = 0xABCD;
        ppu.write(0x2116, 0);
        ppu.write(0x2117, 0);
        assert_eq!(ppu.read(0x2139, 0), 0);
        // Force blank makes VRAM accessible again.
        ppu.write(0x2100, 0x80);
        ppu.write(0x2116, 0);
        ppu.write(0x2117, 0);
        ppu.write(0x2118, 0x12);
        ppu.write(0x2119, 0x34);
        assert_eq!(ppu.vram[0], 0x3412);
    }

    #[test]
    fn cgram_access_is_redirected_during_active_display() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x0F);
        ppu.line = 100;
        ppu.dot = 100; // inside the visible window
        ppu.latch_cgram_address = 7; // the DAC last looked up palette 7
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0x1F);
        ppu.write(0x2122, 0x00);
        assert_eq!(ppu.cgram[7], 0x001F);
        ppu.write(0x2121, 0x02);
        assert_eq!(ppu.read(0x213B, 0), 0x1F);
        assert_eq!(ppu.read(0x213B, 0), 0x00);
        // Outside the visible window the CPU address is used.
        ppu.dot = 10;
        ppu.write(0x2121, 0x02);
        ppu.write(0x2122, 0xE0);
        ppu.write(0x2122, 0x03);
        assert_eq!(ppu.cgram[2], 0x03E0);
    }

    #[test]
    fn forced_blank_at_vblank_resets_the_oam_address() {
        let mut ppu = Ppu::new();
        // ares only resets while force blank is *already* enabled.
        ppu.write(0x2100, 0x80);
        // OAM base = sprite 1 (byte address 4, word address 2).
        ppu.write(0x2102, 2);
        ppu.write(0x2103, 0);
        ppu.write(0x2104, 0x11);
        ppu.write(0x2104, 0x22);
        ppu.write(0x2104, 0x33);
        ppu.write(0x2104, 0x44);
        // Move the address past the sprite and enter forced blank at vblank.
        ppu.write(0x2104, 0x55);
        ppu.line = VBLANK_LINE;
        ppu.write(0x2100, 0x80);
        // $2138 now reads the OAM byte at the reset base (sprite 1's X).
        assert_eq!(ppu.read(0x2138, 0), 0x11);
    }

    #[test]
    fn sprite_overflow_flags_latch_for_the_frame() {
        let mut ppu = Ppu::new();
        ppu.write(0x2100, 0x8F); // force blank (OAM access) + full brightness
        ppu.write(0x2101, 0x20); // OBSEL base size 1: 16x16 / 32x32 sprites
        ppu.write(0x212C, 0x10); // OBJ on the main screen
                                 // 33 sprites on line 100 (a sprite's first visible line is OAM Y + 1).
                                 // OAM addresses are word addresses; a sprite entry is four bytes, so
                                 // sprite n starts at word address n * 2.
        for n in 0..33u16 {
            let word = n * 2;
            ppu.write(0x2102, word as u8);
            ppu.write(0x2103, ((word >> 8) & 1) as u8);
            ppu.write(0x2104, 0);
            ppu.write(0x2104, 99);
            ppu.write(0x2104, 0);
            ppu.write(0x2104, 0);
        }
        // High OAM table: select the large size for every sprite, so 32 stored
        // sprites fetch far more than 34 tiles. The table is written as words
        // (four sprites per byte, two bytes per $2104 pair).
        for byte_index in 0..9u16 {
            let mut value = 0u8;
            for k in 0..4u16 {
                let n = byte_index * 4 + k;
                if n < 33 {
                    value |= 0x02 << ((n & 3) * 2);
                }
            }
            if byte_index % 2 == 0 {
                let word = (0x200 + byte_index) / 2;
                ppu.write(0x2102, word as u8);
                ppu.write(0x2103, ((word >> 8) & 1) as u8);
            }
            ppu.write(0x2104, value);
        }
        ppu.render_line(100, 0);
        let stat = ppu.read(0x213E, 0);
        assert_ne!(stat & 0x40, 0, "rangeOver should be latched");
        // 32 stored 16x16 sprites are 64 tiles, well over the 34-tile limit.
        assert_ne!(stat & 0x80, 0, "timeOver should be latched");
    }

    #[test]
    fn ophct_opvct_return_the_latched_counters() {
        let mut ppu = Ppu::new();
        for _ in 0..75 {
            ppu.tick();
        }
        let dot = ppu.dot();
        let line = ppu.line();
        ppu.latch_counters();
        let h_low = ppu.read(0x213C, 0);
        let h_high = ppu.read(0x213C, 0);
        let v_low = ppu.read(0x213D, 0);
        let v_high = ppu.read(0x213D, 0);
        assert_eq!(u16::from(h_low) | (u16::from(h_high & 1) << 8), dot);
        assert_eq!(u16::from(v_low) | (u16::from(v_high & 1) << 8), line);
        // $213F resets the byte latches and reports the counter-latch status.
        let stat = ppu.read(0x213F, 0);
        assert_eq!(stat & 0x0F, 0x03);
        assert_ne!(stat & 0x40, 0);
        let stat = ppu.read(0x213F, 0);
        assert_eq!(stat & 0x40, 0);
    }

    #[test]
    fn stat78_reports_the_pal_region_bit() {
        let mut ppu = Ppu::new();
        ppu.write(0x2133, 0);
        let ntsc = ppu.read(0x213F, 0);
        assert_eq!(ntsc & 0x10, 0);
        ppu.set_region(crate::rom::Region::Pal);
        let pal = ppu.read(0x213F, 0);
        assert_ne!(pal & 0x10, 0);
    }

    #[test]
    fn pal_interlaced_line_311_is_342_dots() {
        let mut ppu = Ppu::new();
        ppu.set_region(crate::rom::Region::Pal);
        ppu.setini = 0x01; // SETINI interlace
        ppu.frame_interlace = true;
        ppu.field = true;
        ppu.frame_lines = 312;
        ppu.line = 310;
        ppu.line_dots = DOTS_PER_LINE;
        for _ in 0..DOTS_PER_LINE {
            ppu.tick();
        }
        assert_eq!(ppu.line, 311);
        assert_eq!(ppu.line_dots, DOTS_PER_LINE + 1);
        // The long line is the last of the 312-line field, so it wraps to 0 and
        // the next field is the 313-line one.
        for _ in 0..DOTS_PER_LINE + 1 {
            ppu.tick();
        }
        assert_eq!(ppu.line, 0);
        assert!(!ppu.field);
        assert_eq!(ppu.frame_lines, 313);
        assert_eq!(ppu.line_dots, DOTS_PER_LINE);
    }
}
