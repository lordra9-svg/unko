//! S-PPU1/S-PPU2 line-based renderer.
//!
//! The PPU register map, VRAM/CGRAM/OAM access, vblank/NMI timing and
//! background/sprite rendering are implemented here. The renderer produces a
//! 256x224 RGBA8888 frame. Colour math, windows, interlace, hires output and
//! cycle-exact background fetch timing are still approximations; the public
//! base modes 0-7 and sprites are covered.

/// Visible frame width.
pub const SCREEN_WIDTH: usize = 256;
/// Visible frame height.
pub const SCREEN_HEIGHT: usize = 224;
/// RGBA8888 frame size.
pub const FRAME_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 4;

const DOTS_PER_LINE: u16 = 341;
const LINES_PER_FRAME: u16 = 262;
const VBLANK_LINE: u16 = 225;

/// The S-PPU pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ppu {
    vram: Box<[u16; 0x8000]>,
    cgram: [u16; 256],
    oam: [u8; 0x220],
    framebuffer: Box<[u8; FRAME_SIZE]>,
    line: u16,
    dot: u16,
    field: bool,
    vblank: bool,
    nmi: bool,
    frame_ready: bool,
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
    /// $2137 latch
    latched_h: u16,
    latched_v: u16,
    counter_latch: bool,
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
            vram: Box::new([0; 0x8000]),
            cgram: [0; 256],
            oam: [0; 0x220],
            framebuffer: Box::new([0; FRAME_SIZE]),
            line: 0,
            dot: 0,
            field: false,
            vblank: false,
            nmi: false,
            frame_ready: false,
            display_brightness: 0,
            display_disable: false,
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
            counter_latch: false,
            line_event: false,
            layer_mask: None,
        }
    }

    /// Reset the PPU.
    pub fn reset(&mut self) {
        *self = Self::new();
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
        self.render_sprites(&mut main, &mut sub, y, &obj_priority);
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

    /// Read a CGRAM word (debug/test helper).
    pub fn cgram_word(&self, address: u8) -> u16 {
        self.cgram[usize::from(address)]
    }

    /// Current background mode (debug).
    pub const fn bg_mode(&self) -> u8 {
        self.bg_mode
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
    /// ares reports `hcounter <= 2 || hcounter >= 1096` on its 1365-clock
    /// line; a dot is four of those clocks.
    pub const fn in_hblank(&self) -> bool {
        self.dot == 0 || self.dot >= 271
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

    /// The RGBA8888 frame buffer.
    pub fn frame_buffer(&self) -> &[u8] {
        &self.framebuffer[..]
    }

    /// Mark the current frame as presented.
    pub fn acknowledge_frame(&mut self) {
        self.frame_ready = false;
    }

    /// Advance one PPU dot.
    pub fn tick(&mut self) {
        self.dot += 1;
        if self.dot < DOTS_PER_LINE {
            return;
        }
        self.dot = 0;
        self.line += 1;
        self.line_event = true;
        if self.line >= LINES_PER_FRAME {
            self.line = 0;
            self.field = !self.field;
        }
        match self.line {
            0 => self.vblank = false,
            VBLANK_LINE => {
                self.vblank = true;
                self.nmi = true;
            }
            240 => self.frame_ready = true,
            _ => {}
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
    /// The PPU numbers visible NTSC lines 1-224; line 0 is part of vblank and
    /// never produces pixels. Background and sprite fetches therefore use the
    /// line number itself, not the framebuffer row.
    pub fn render_current_line(&mut self) {
        if (1..=SCREEN_HEIGHT as u16).contains(&self.line) {
            self.render_line(usize::from(self.line), usize::from(self.line - 1));
        }
    }

    /// Read a PPU register in `$2100-$213F`.
    pub fn read(&mut self, address: u16) -> u8 {
        match address {
            0x2134 => (self.m7a.wrapping_mul(self.m7b >> 8) as u32 & 0xFF) as u8,
            0x2135 => ((self.m7a.wrapping_mul(self.m7b >> 8) as u32 >> 8) & 0xFF) as u8,
            0x2136 => ((self.m7a.wrapping_mul(self.m7b >> 8) as u32 >> 16) & 0xFF) as u8,
            0x2137 => 0,
            0x2138 => {
                let value = self.read_oam(self.oam_address);
                self.oam_address = (self.oam_address + 1) & 0x3FF;
                self.set_first_sprite();
                value
            }
            0x2139 => {
                let low = self.vram_read_latch as u8;
                if self.vmain & 0x80 == 0 {
                    self.vram_read_latch =
                        self.vram[usize::from(self.vram_word_address() & 0x7FFF)];
                    self.vram_address = self.vram_address.wrapping_add(self.vram_increment());
                }
                low
            }
            0x213A => {
                let high = (self.vram_read_latch >> 8) as u8;
                if self.vmain & 0x80 != 0 {
                    self.vram_read_latch =
                        self.vram[usize::from(self.vram_word_address() & 0x7FFF)];
                    self.vram_address = self.vram_address.wrapping_add(self.vram_increment());
                }
                high
            }
            0x213B => {
                let address = usize::from(self.cgram_address);
                if self.cgram_latch {
                    self.cgram_latch = false;
                    self.cgram_address = self.cgram_address.wrapping_add(1);
                    (self.cgram[address] >> 8) as u8
                } else {
                    self.cgram_latch = true;
                    self.cgram[address] as u8
                }
            }
            0x213C => {
                if self.counter_latch {
                    self.counter_latch = false;
                    (self.latched_h >> 8) as u8
                } else {
                    self.counter_latch = true;
                    self.latched_h as u8
                }
            }
            0x213D => {
                if self.counter_latch {
                    self.counter_latch = false;
                    (self.latched_v >> 8) as u8
                } else {
                    self.counter_latch = true;
                    self.latched_v as u8
                }
            }
            0x213E => 0x01,
            0x213F => 0x03 | (u8::from(self.field) << 7),
            _ => 0,
        }
    }

    /// Write a PPU register in `$2100-$213F`.
    pub fn write(&mut self, address: u16, data: u8) {
        match address {
            0x2100 => {
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
            0x2116 => self.vram_address = (self.vram_address & 0xFF00) | u16::from(data),
            0x2117 => self.vram_address = (self.vram_address & 0x00FF) | (u16::from(data) << 8),
            0x2118 => self.write_vram_byte(data, false),
            0x2119 => self.write_vram_byte(data, true),
            0x211A => self.m7sel = data,
            0x211B => self.m7a = (self.m7a & 0x00FF) | (i16::from(data as i8) << 8),
            0x211C => self.m7b = (self.m7b & 0x00FF) | (i16::from(data as i8) << 8),
            0x211D => self.m7c = (self.m7c & 0x00FF) | (i16::from(data as i8) << 8),
            0x211E => self.m7d = (self.m7d & 0x00FF) | (i16::from(data as i8) << 8),
            0x211F => self.m7x = (self.m7x & 0x00FF) | (i16::from(data as i8) << 8),
            0x2120 => self.m7y = (self.m7y & 0x00FF) | (i16::from(data as i8) << 8),
            0x2121 => {
                self.cgram_address = data;
                self.cgram_latch = false;
            }
            0x2122 => {
                let address = usize::from(self.cgram_address);
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
            0x2137 => {
                self.latched_h = self.dot;
                self.latched_v = self.line;
            }
            _ => {}
        }
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
        let word = self.vram[usize::from(address)];
        self.vram[usize::from(address)] = if high {
            (word & 0x00FF) | (u16::from(data) << 8)
        } else {
            (word & 0xFF00) | u16::from(data)
        };
        let increment = self.vram_increment();
        if (high && self.vmain & 0x80 != 0) || (!high && self.vmain & 0x80 == 0) {
            self.vram_address = self.vram_address.wrapping_add(increment);
        }
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

    fn read_oam(&self, address: u16) -> u8 {
        self.oam[Self::oam_index(address)]
    }

    fn write_oam(&mut self, address: u16, data: u8) {
        let index = Self::oam_index(address);
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
            if mode == 7 && layer == 0 {
                self.render_mode7(&mut main, &mut sub, y, layer);
            } else if mode <= 6 {
                self.render_background(&mut main, &mut sub, y, layer, mode, priority);
            }
        }
        if main_mask & 0x10 != 0 || self.ts & 0x10 != 0 {
            self.render_sprites(&mut main, &mut sub, y, &obj_priority);
        }
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
        let screen_h = 32u32 << ((sc as u32) & 1);
        let screen_v = 32u32 << (((sc as u32) >> 1) & 1);
        let hsize = (screen_h * 8) << tile_size;
        let vsize = (screen_v * 8) << tile_size;
        let hscroll = u32::from(self.bg_h[layer]);
        let vscroll = u32::from(self.bg_v[layer]);
        let screen_base = (u32::from(sc) & 0xFC) << 8;
        let char_base = (u32::from(self.bg_nba[layer]) & 0x0F) << 12;
        let palette_offset = if mode == 0 { (layer as u16) << 5 } else { 0 };
        let palette_shift = 2u16 << bpp_shift;
        let tile_stride = 8u32 << bpp_shift;
        let main_enable = self.layer_mask.unwrap_or(self.tm) & (1 << layer) != 0;
        let sub_enable = self.ts & (1 << layer) != 0 && self.layer_mask.is_none();

        for x in 0..SCREEN_WIDTH {
            let h = (x as u32 + hscroll) & (hsize - 1);
            let v = (y as u32 + vscroll) & (vsize - 1);
            let htile = h >> (3 + tile_size);
            let vtile = v >> (3 + tile_size);
            let mut offset = (htile & 0x1F) | ((vtile & 0x1F) << 5);
            if htile & 0x20 != 0 {
                offset += 0x400;
            }
            if vtile & 0x20 != 0 {
                offset += if screen_h > 32 { 0x800 } else { 0x400 };
            }
            let attributes = self.vram[((screen_base + offset) & 0x7FFF) as usize];
            let mut character = attributes & 0x03FF;
            let hflip = attributes & 0x4000 != 0;
            let vflip = attributes & 0x8000 != 0;
            let mut row = v & 7;
            if vflip {
                row = 7 - row;
            }
            if tile_size == 1 {
                if ((h >> 3) & 1 != 0) != hflip {
                    character += 1;
                }
                if ((v >> 3) & 1 != 0) != vflip {
                    character += 16;
                }
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
                continue;
            }
            let palette_group = (attributes >> 10) & 0x07;
            let palette = palette_offset + (palette_group << palette_shift) + color;
            let pixel = Pixel {
                color: palette as u8,
                priority: priority[usize::from(attributes & 0x2000 != 0)],
                layer: layer as u8,
            };
            if main_enable && !(self.tmw & (1 << layer) != 0 && self.window_masks(layer, x)) {
                plot(main, x, pixel);
            }
            if sub_enable && !(self.tsw & (1 << layer) != 0 && self.window_masks(layer, x)) {
                plot(sub, x, pixel);
            }
        }
    }

    fn render_mode7(
        &self,
        main: &mut [Pixel; SCREEN_WIDTH],
        sub: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        layer: usize,
    ) {
        let a = i32::from(self.m7a);
        let b = i32::from(self.m7b);
        let c = i32::from(self.m7c);
        let d = i32::from(self.m7d);
        let hcenter = sign13(self.m7x);
        let vcenter = sign13(self.m7y);
        let hoffset = sign13(self.m7_hofs as i16);
        let voffset = sign13(self.m7_vofs as i16);
        let y = if self.m7sel & 0x02 != 0 {
            255 - y as i32
        } else {
            y as i32
        };
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
        let extbg = self.setini & 0x40 != 0;
        for x in 0..SCREEN_WIDTH {
            let x = if self.m7sel & 0x01 != 0 {
                255 - x as i32
            } else {
                x as i32
            };
            let pixel_x = (origin_x + a * x) >> 8;
            let pixel_y = (origin_y + c * x) >> 8;
            let out_of_bounds = (pixel_x | pixel_y) & !1023 != 0;
            if out_of_bounds && self.m7sel & 0x80 != 0 {
                continue;
            }
            let tile_x = (pixel_x >> 3) & 0x7F;
            let tile_y = (pixel_y >> 3) & 0x7F;
            let tile = self.vram[((tile_y << 7 | tile_x) & 0x7FFF) as usize] as u8;
            let palette_address = ((pixel_y & 7) << 3 | (pixel_x & 7)) as usize;
            let index = usize::from((tile as u16) << 6 | palette_address as u16);
            let mut palette = (self.vram[index & 0x7FFF] >> 8) as u8;
            if palette == 0 {
                continue;
            }
            let priority = if layer == 0 || !extbg {
                2
            } else {
                let bit = palette >> 7;
                palette &= 0x7F;
                if bit != 0 {
                    5
                } else {
                    1
                }
            };
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

    fn render_sprites(
        &self,
        main: &mut [Pixel; SCREEN_WIDTH],
        sub: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        priority: &[u8; 4],
    ) {
        let base_size = (self.obsel >> 5) & 0x07;
        let nameselect = (self.obsel >> 3) & 0x03;
        let tiledata = u32::from(self.obsel & 0x07) << 13;
        let main_enable = self.layer_mask.unwrap_or(self.tm) & 0x10 != 0;
        let sub_enable = self.ts & 0x10 != 0 && self.layer_mask.is_none();
        // Sprites are evaluated from `firstSprite` upward; the earliest one
        // wins an equal-priority overlap.
        for k in 0..128 {
            let sprite = (usize::from(self.first_sprite) + k) & 0x7F;
            let base = sprite * 4;
            let attr = self.oam[base + 3];
            let high = self.oam[0x200 + sprite / 4];
            let high_bit = (sprite & 3) * 2;
            let size = (high >> (high_bit + 1)) & 1;
            let width = sprite_size(base_size, size, false);
            let height = sprite_size(base_size, size, true);
            let x = u16::from(self.oam[base]) | (u16::from((high >> high_bit) & 1) << 8);
            let y0 = u16::from(self.oam[base + 1]);
            // Hardware quirk: a sprite's first visible line is OAM Y + 1.
            let first = usize::from(y0) + 1;
            if y < first || y >= first + usize::from(height) {
                continue;
            }
            let row = if attr & 0x80 != 0 {
                height - 1 - (y as u16 - first as u16)
            } else {
                y as u16 - first as u16
            };
            let character = self.oam[base + 2];
            let tile_width = width / 8;
            let priority = priority[usize::from((attr >> 4) & 0x03)];
            for tx in 0..tile_width {
                let sx = x + tx * 8;
                for px in 0..8 {
                    let screen_x = sx + px;
                    if screen_x >= SCREEN_WIDTH as u16 {
                        continue;
                    }
                    let chrx = (u16::from(character) & 0x0F)
                        + if attr & 0x40 != 0 {
                            tile_width - 1 - tx
                        } else {
                            tx
                        };
                    // Object tiles live in a 16-column grid: the row nibble
                    // selects a 256-word line and the column nibble a 16-word
                    // tile, matching ares `Object::fetch`.
                    let chry = ((u16::from(character) >> 4) + row / 8) & 0x0F;
                    let chrx = chrx & 0x0F;
                    let mut address = tiledata
                        + (u32::from(chry) << 8)
                        + (u32::from(chrx) << 4)
                        + u32::from(row & 7);
                    if attr & 0x01 != 0 {
                        address += (u32::from(nameselect) + 1) << 12;
                    }
                    // Horizontal flip reverses the tile order and reads each
                    // tile from bit 0 upward; it does not mirror the plane
                    // bits a second time.
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
                        priority,
                        layer: 4,
                    };
                    let index = usize::from(screen_x);
                    let masked = self.window_masks(4, index);
                    if main_enable && !(self.tmw & 0x10 != 0 && masked) {
                        plot_sprite(main, index, pixel);
                    }
                    if sub_enable && !(self.tsw & 0x10 != 0 && masked) {
                        plot_sprite(sub, index, pixel);
                    }
                }
            }
        }
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
        for x in 0..SCREEN_WIDTH {
            let above = main[x];
            let below = sub[x];
            let backdrop = above.priority == 0;
            let layer = if backdrop { 5 } else { above.layer };
            let mut color = if backdrop {
                self.cgram[0]
            } else {
                self.cgram[usize::from(above.color)]
            };
            let sub_transparent = below.priority == 0;
            let sub_color = if sub_transparent {
                self.cgram[0]
            } else {
                self.cgram[usize::from(below.color)]
            };
            let (colour_above, colour_below) = self.colour_window(x);
            let enable = (if backdrop {
                self.cgadsub & 0x20 != 0
            } else {
                self.cgadsub & (1 << layer) != 0
            }) && colour_above
                && colour_below;
            if enable {
                let use_sub = self.cgwsel & 0x02 != 0 && !sub_transparent;
                let operand = if use_sub { sub_color } else { fixed };
                let halve = self.cgadsub & 0x40 != 0 && use_sub;
                color = blend(color, operand, halve, self.cgadsub & 0x80 != 0);
            }
            // 5-bit to 16-bit normalization (ares image::normalize), then the
            // DAC luma and the high byte for the 8-bit frame buffer.
            let scale = |value: u16| {
                let value = u32::from(value & 0x1F) * 65_535 / 31;
                ((value * num / den) >> 8) as u8
            };
            let index = (y * SCREEN_WIDTH + x) * 4;
            self.framebuffer[index] = scale(color);
            self.framebuffer[index + 1] = scale(color >> 5);
            self.framebuffer[index + 2] = scale(color >> 10);
            self.framebuffer[index + 3] = 0xFF;
        }
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

/// Plot a background pixel: higher priority wins, ties keep the earlier layer.
fn plot(buffer: &mut [Pixel; SCREEN_WIDTH], x: usize, pixel: Pixel) {
    if pixel.priority == 0 {
        return;
    }
    if pixel.priority > buffer[x].priority {
        buffer[x] = pixel;
    }
}

/// Plot a sprite pixel: later sprites win ties within the object layer, but a
/// sprite never displaces a background pixel of equal priority.
fn plot_sprite(buffer: &mut [Pixel; SCREEN_WIDTH], x: usize, pixel: Pixel) {
    if pixel.priority == 0 {
        return;
    }
    let current = buffer[x];
    if pixel.priority > current.priority
        || (current.layer == 4 && pixel.priority == current.priority)
    {
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
}
