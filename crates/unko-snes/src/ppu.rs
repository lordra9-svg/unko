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
    /// $2123-$212F windows and main/sub screens
    window: [u8; 6],
    tm: u8,
    ts: u8,
    tmw: u8,
    tsw: u8,
    /// $2130-$2132 colour math
    cgwsel: u8,
    cgadsub: u8,
    coldata: u8,
    /// $2133 SETINI
    setini: u8,
    /// $2137 latch
    latched_h: u16,
    latched_v: u16,
    counter_latch: bool,
    line_event: bool,
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
            window: [0; 6],
            tm: 0,
            ts: 0,
            tmw: 0,
            tsw: 0,
            cgwsel: 0,
            cgadsub: 0,
            coldata: 0,
            setini: 0,
            latched_h: 0,
            latched_v: 0,
            counter_latch: false,
            line_event: false,
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
    pub fn render_current_line(&mut self) {
        if (1..=SCREEN_HEIGHT as u16).contains(&self.line) {
            self.render_line(usize::from(self.line - 1));
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
            0x213E => (self.obsel & 0x80) | 0x03,
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
            0x2102 => self.oam_base = (self.oam_base & 0x100) | u16::from(data),
            0x2103 => {
                self.oam_base = (self.oam_base & 0x0FF) | (u16::from(data & 0x01) << 8);
                self.oam_address = self.oam_base;
            }
            0x2104 => {
                self.write_oam(self.oam_address, data);
                self.oam_address = (self.oam_address + 1) & 0x3FF;
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
            0x2123..=0x212B => self.window[usize::from((address - 0x2123) / 2)] = data,
            0x212C => self.tm = data,
            0x212D => self.ts = data,
            0x212E => self.tmw = data,
            0x212F => self.tsw = data,
            0x2130 => self.cgwsel = data,
            0x2131 => self.cgadsub = data,
            0x2132 => self.coldata = data,
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

    fn read_oam(&self, address: u16) -> u8 {
        self.oam[usize::from(address & 0x3FF)]
    }

    fn write_oam(&mut self, address: u16, data: u8) {
        self.oam[usize::from(address & 0x3FF)] = data;
    }

    fn render_line(&mut self, y: usize) {
        let mut pixels = [Pixel::default(); SCREEN_WIDTH];
        let mode = self.bg_mode & 0x07;
        let order: &[usize] = match mode {
            0 => &[3, 2, 1, 0],
            1 => &[2, 1, 0],
            2..=4 => &[1, 0],
            5..=7 => &[0],
            _ => &[],
        };
        let main_screen = self.tm;
        for &layer in order {
            if main_screen & (1 << layer) != 0 {
                if mode == 7 && layer == 0 {
                    self.render_mode7(&mut pixels, y);
                } else if mode <= 6 {
                    self.render_background(&mut pixels, y, layer, mode);
                }
            }
        }
        if main_screen & 0x10 != 0 {
            self.render_sprites(&mut pixels, y);
        }
        self.present_line(y, &pixels);
    }

    fn render_background(
        &self,
        pixels: &mut [Pixel; SCREEN_WIDTH],
        y: usize,
        layer: usize,
        mode: u8,
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

        for (x, pixel) in pixels.iter_mut().enumerate() {
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
            let bit = if hflip {
                x as u32 & 7
            } else {
                7 - (x as u32 & 7)
            };
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
            let priority = ((4 - layer) as u8) << 4 | (u8::from(attributes & 0x2000 != 0) << 3);
            if priority >= pixel.priority {
                *pixel = Pixel {
                    color: palette,
                    priority,
                };
            }
        }
    }

    fn render_mode7(&self, pixels: &mut [Pixel; SCREEN_WIDTH], y: usize) {
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
        for (x, pixel) in pixels.iter_mut().enumerate() {
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
            let palette = (self.vram[index & 0x7FFF] >> 8) as u8;
            if palette == 0 {
                continue;
            }
            let priority = 0x48
                | (u8::from(self.cgadsub & 0x20 != 0) << 2)
                | (u8::from(self.cgadsub & 0x40 != 0) << 1);
            if priority >= pixel.priority {
                *pixel = Pixel {
                    color: u16::from(palette),
                    priority,
                };
            }
        }
    }

    fn render_sprites(&self, pixels: &mut [Pixel; SCREEN_WIDTH], y: usize) {
        let base_size = (self.obsel >> 5) & 0x07;
        let nameselect = (self.obsel >> 3) & 0x03;
        let tiledata = u32::from(self.obsel & 0x07) << 13;
        for sprite in 0..128 {
            let base = sprite * 4;
            let attr = self.oam[base + 3];
            let high = self.oam[0x200 + sprite / 4];
            let high_bit = (sprite & 3) * 2;
            let size = (high >> (high_bit + 1)) & 1;
            let width = sprite_size(base_size, size, false);
            let height = sprite_size(base_size, size, true);
            let x = u16::from(self.oam[base]) | (u16::from((high >> high_bit) & 1) << 8);
            let y0 = u16::from(self.oam[base + 1]);
            if y < usize::from(y0) || y >= usize::from(y0) + usize::from(height) {
                continue;
            }
            let row = if attr & 0x80 != 0 {
                height - 1 - (y as u16 - y0)
            } else {
                y as u16 - y0
            };
            let character = self.oam[base + 2];
            let tile_width = width / 8;
            for tx in 0..tile_width {
                let sx = x + tx * 8;
                for px in 0..8 {
                    let screen_x = sx + px;
                    if screen_x >= SCREEN_WIDTH as u16 {
                        continue;
                    }
                    let mx = if attr & 0x40 != 0 { 7 - px } else { px };
                    let chrx = (u16::from(character) & 0x0F)
                        + if attr & 0x40 != 0 {
                            tile_width - 1 - tx
                        } else {
                            tx
                        };
                    let chry = ((u16::from(character) >> 4) + row / 8) & 0x0F;
                    let mut address = tiledata
                        + ((u32::from(chry) + (u32::from(chrx) & 0x0F)) << 4)
                        + u32::from(row & 7);
                    if attr & 0x01 != 0 {
                        address += (u32::from(nameselect) + 1) << 12;
                    }
                    let bit = if attr & 0x40 != 0 { mx } else { 7 - mx };
                    let word0 = self.vram[(address & 0x7FFF) as usize];
                    let word1 = self.vram[((address + 8) & 0x7FFF) as usize];
                    let color = ((word0 >> bit) & 1)
                        | (((word0 >> (8 + bit)) & 1) << 1)
                        | (((word1 >> bit) & 1) << 2)
                        | (((word1 >> (8 + bit)) & 1) << 3);
                    if color == 0 {
                        continue;
                    }
                    let palette = 128 + (u16::from((attr >> 1) & 0x07) << 4) + color;
                    let priority = 0x60u8
                        .wrapping_sub(((attr >> 4) & 0x03) << 4)
                        .wrapping_add(4);
                    let pixel = &mut pixels[usize::from(screen_x)];
                    if priority >= pixel.priority {
                        *pixel = Pixel {
                            color: palette,
                            priority,
                        };
                    }
                }
            }
        }
    }

    fn present_line(&mut self, y: usize, pixels: &[Pixel; SCREEN_WIDTH]) {
        let brightness = if self.display_disable {
            0
        } else {
            u32::from(self.display_brightness) + 1
        };
        for (x, pixel) in pixels.iter().enumerate() {
            let color = if pixel.color == 0 {
                self.cgram[0]
            } else {
                self.cgram[usize::from(pixel.color & 0xFF)]
            };
            let expand = |value: u16| {
                let value = value & 0x1F;
                (u32::from(value) << 3) | u32::from(value >> 2)
            };
            let r = expand(color);
            let g = expand(color >> 5);
            let b = expand(color >> 10);
            let scale = |value: u32| ((value * brightness) >> 4).min(255) as u8;
            let index = (y * SCREEN_WIDTH + x) * 4;
            self.framebuffer[index] = scale(r);
            self.framebuffer[index + 1] = scale(g);
            self.framebuffer[index + 2] = scale(b);
            self.framebuffer[index + 3] = 0xFF;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Pixel {
    color: u16,
    priority: u8,
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
        ppu.render_line(0);
        assert_eq!(&ppu.framebuffer[0..4], &[0, 255, 0, 255]);
    }
}
