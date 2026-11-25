//! RDP scaffold: the DPC register block and a command interpreter.
//!
//! Commands are fetched as big-endian 64-bit words from RDRAM between
//! `DPC_START` and `DPC_END`. Implemented: `SET_COLOR_IMAGE`,
//! `SET_FILL_COLOR`, `FILL_RECTANGLE`, `SET_OTHER_MODES` (1/2-cycle copy and
//! alpha blender, coverage, dither), the two-cycle combiner, texture sampling
//! (RGBA/IA/I/CI with TLUT) and the triangle edgewalker. Depth testing and
//! tile loading are later milestones.

/// Texture image descriptor (`SET_TEXTURE_IMAGE`).
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // format/size/width are recorded for the load path.
struct TextureImage {
    format: u8,
    size: u8,
    width: u16,
    address: u32,
}

/// Tile descriptor (`SET_TILE`).
#[derive(Debug, Clone, Copy, Default)]
struct Tile {
    format: u8,
    size: u8,
    line: u16,
    address: u16,
    t_clamp: bool,
    t_mirror: bool,
    t_mask: u8,
    s_clamp: bool,
    s_mirror: bool,
    s_mask: u8,
    s_shift: u8,
    t_shift: u8,
}

/// Tile bounds (`SET_TILE_SIZE`).
#[derive(Debug, Clone, Copy, Default)]
struct TileSize {
    s_low: u16,
    t_low: u16,
    s_high: u16,
    t_high: u16,
}

/// `SET_OTHER_MODES`: cycle type, coverage and the per-cycle blender selectors.
#[derive(Debug, Clone, Copy, Default)]
struct OtherModes {
    /// 0 = 1 cycle, 1 = 2 cycles, 2 = copy, 3 = fill.
    cycle: u8,
    force_blend: bool,
    alpha_cvg_select: bool,
    cvg_times_alpha: bool,
    color_on_coverage: bool,
    #[allow(dead_code)] // image-read coverage blending is a later milestone.
    image_read: bool,
    aa: bool,
    dither: u8,
    coverage_mode: u8,
    /// `[cycle][1a, 1b, 2a, 2b]`.
    blend: [[u8; 4]; 2],
}

/// Interpolated texture coordinates (16.16 fixed point).
#[derive(Debug, Clone, Copy, Default)]
struct TexCoords {
    s: [i32; 4],
    t: [i32; 4],
    #[allow(dead_code)] // perspective W is interpolated for a later milestone.
    w: [i32; 4],
}

/// DPC register block.
#[derive(Debug, Clone)]
pub struct Rdp {
    /// `DPC_START`.
    pub start: u32,
    /// `DPC_END`.
    pub end: u32,
    /// `DPC_CURRENT`.
    pub current: u32,
    /// `DPC_STATUS`.
    pub status: u32,
    color_address: u32,
    color_width: u32,
    color_format: u8,
    color_size: u8,
    fill_color: u32,
    primitive_color: u32,
    environment_color: u32,
    combine_mode: u64,
    blend_color: u32,
    fog_color: u32,
    other_modes: OtherModes,
    triangles: u64,
    texture: TextureImage,
    tiles: [Tile; 8],
    tile_sizes: [TileSize; 8],
    tlut: [u16; 256],
    tlut_valid: bool,
}

impl Default for Rdp {
    fn default() -> Self {
        Self::new()
    }
}

impl Rdp {
    /// Creates an idle RDP.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            current: 0,
            status: 0,
            color_address: 0,
            color_width: 320,
            color_format: 2,
            color_size: 2,
            fill_color: 0,
            primitive_color: 0,
            environment_color: 0,
            combine_mode: 0,
            blend_color: 0,
            fog_color: 0,
            other_modes: OtherModes::default(),
            triangles: 0,
            texture: TextureImage::default(),
            tiles: [Tile::default(); 8],
            tile_sizes: [TileSize::default(); 8],
            tlut: [0; 256],
            tlut_valid: false,
        }
    }

    /// Reads a DPC register (`0x0410_0000 + offset`).
    #[must_use]
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset & 0x1F {
            0x00 => self.start,
            0x04 => self.end,
            0x08 => self.current,
            0x0C => self.status,
            _ => 0,
        }
    }

    /// Writes a DPC register; writing `DPC_END` processes the command list.
    pub fn write_register(&mut self, offset: u32, value: u32, rdram: &mut [u8]) {
        match offset & 0x1F {
            0x00 => self.start = value,
            0x04 => {
                self.end = value;
                self.current = self.start;
                self.process(rdram);
            }
            0x08 => self.current = value,
            0x0C => self.status = value,
            _ => {}
        }
    }

    /// Framebuffer address and 16-bit row width.
    #[must_use]
    pub fn color_image(&self) -> (u32, u32) {
        (self.color_address, self.color_width)
    }

    /// Current primitive colour (RGBA8888, set by `SET_PRIMITIVE_COLOR`).
    #[must_use]
    pub fn primitive_color(&self) -> u32 {
        self.primitive_color
    }

    /// Current environment colour (RGBA8888).
    #[must_use]
    pub fn environment_color(&self) -> u32 {
        self.environment_color
    }

    /// Raw combiner configuration (`SET_COMBINE_MODE`).
    #[must_use]
    pub fn combine_mode(&self) -> u64 {
        self.combine_mode
    }

    /// Number of triangle commands seen (rasterisation is not implemented yet).
    #[must_use]
    pub fn triangles(&self) -> u64 {
        self.triangles
    }

    fn process(&mut self, rdram: &mut [u8]) {
        while self.current.wrapping_add(8) <= self.end {
            let address = self.current as usize;
            let Some(bytes) = rdram.get(address..address + 8) else {
                break;
            };
            let mut word = [0u8; 8];
            word.copy_from_slice(bytes);
            let command = u64::from_be_bytes(word);
            let opcode = (command >> 56) as u8;
            match opcode {
                0x36 => self.fill_rectangle(command, rdram),
                0x37 => self.fill_color = command as u32,
                0x30 => {
                    let s_low = ((command >> 44) & 0xFFF) as usize;
                    let t_low = ((command >> 32) & 0xFFF) as usize;
                    let s_high = ((command >> 12) & 0xFFF) as usize;
                    let count = (s_high.saturating_sub(s_low) + 1).min(256);
                    let tile = self.tiles[0];
                    let base = self.texture.address as usize
                        + t_low * usize::from(tile.line) * 8
                        + s_low * 2;
                    for (index, slot) in self.tlut.iter_mut().take(count).enumerate() {
                        let offset = base + index * 2;
                        if let Some(bytes) = rdram.get(offset..offset + 2) {
                            *slot = u16::from_be_bytes([bytes[0], bytes[1]]);
                        }
                    }
                    self.tlut_valid = true;
                }
                0x32 => {
                    let index = ((command >> 24) & 7) as usize;
                    self.tile_sizes[index] = TileSize {
                        s_low: ((command >> 44) & 0xFFF) as u16,
                        t_low: ((command >> 32) & 0xFFF) as u16,
                        s_high: ((command >> 12) & 0xFFF) as u16,
                        t_high: (command & 0xFFF) as u16,
                    };
                }
                0x35 => {
                    let index = ((command >> 24) & 7) as usize;
                    self.tiles[index] = Tile {
                        format: ((command >> 53) & 7) as u8,
                        size: ((command >> 51) & 3) as u8,
                        line: ((command >> 41) & 0x1FF) as u16,
                        address: ((command >> 32) & 0x1FF) as u16,
                        t_clamp: command & (1 << 19) != 0,
                        t_mirror: command & (1 << 18) != 0,
                        t_mask: ((command >> 14) & 0xF) as u8,
                        s_clamp: command & (1 << 9) != 0,
                        s_mirror: command & (1 << 8) != 0,
                        s_mask: ((command >> 4) & 0xF) as u8,
                        s_shift: (command & 0xF) as u8,
                        t_shift: ((command >> 10) & 0xF) as u8,
                    };
                }
                0x3A => self.primitive_color = command as u32,
                0x3B => self.environment_color = command as u32,
                0x3C => self.combine_mode = command,
                0x39 => self.blend_color = command as u32,
                0x38 => self.fog_color = command as u32,
                0x2F => {
                    let high = (command >> 32) as u32;
                    let low = command as u32;
                    self.other_modes = OtherModes {
                        cycle: ((high >> 20) & 3) as u8,
                        dither: ((high >> 4) & 0xF) as u8,
                        force_blend: low & (1 << 14) != 0,
                        alpha_cvg_select: low & (1 << 13) != 0,
                        cvg_times_alpha: low & (1 << 12) != 0,
                        color_on_coverage: low & (1 << 7) != 0,
                        image_read: low & (1 << 6) != 0,
                        aa: low & (1 << 3) != 0,
                        coverage_mode: ((low >> 8) & 3) as u8,
                        blend: [
                            [
                                ((low >> 30) & 3) as u8,
                                ((low >> 26) & 3) as u8,
                                ((low >> 22) & 3) as u8,
                                ((low >> 18) & 3) as u8,
                            ],
                            [
                                ((low >> 28) & 3) as u8,
                                ((low >> 24) & 3) as u8,
                                ((low >> 20) & 3) as u8,
                                ((low >> 16) & 3) as u8,
                            ],
                        ],
                    };
                }
                0x3D => {
                    self.texture = TextureImage {
                        format: ((command >> 53) & 7) as u8,
                        size: ((command >> 51) & 3) as u8,
                        width: ((command >> 32) & 0x3FF) as u16,
                        address: (command & 0x03FF_FFFF) as u32,
                    };
                }
                0x08..=0x0F => {
                    self.triangles += 1;
                    let shaded = opcode >= 0x0C;
                    let textured = matches!(opcode, 0x0A | 0x0B | 0x0E | 0x0F);
                    let words = if shaded && textured {
                        20
                    } else if shaded || textured {
                        12
                    } else {
                        4
                    };
                    let start = self.current as usize;
                    let mut edge = [0u64; 4];
                    for (index, slot) in edge.iter_mut().enumerate() {
                        let offset = start + index * 8;
                        let Some(bytes) = rdram.get(offset..offset + 8) else {
                            break;
                        };
                        let mut word = [0u8; 8];
                        word.copy_from_slice(bytes);
                        *slot = u64::from_be_bytes(word);
                    }
                    let (shade_offset, texture_offset) = match (textured, shaded) {
                        (true, true) => (4, 12),
                        (true, false) => (0, 4),
                        (false, true) => (4, 0),
                        (false, false) => (0, 0),
                    };
                    let shade = if shaded {
                        read_word(rdram, start + shade_offset * 8)
                            .map(shade_color)
                            .unwrap_or(0xFFFF_FFFF)
                    } else {
                        0xFFFF_FFFF
                    };
                    let coords = if textured {
                        Some(parse_texcoords(rdram, start + texture_offset * 8))
                    } else {
                        None
                    };
                    let base_color = if shaded { shade } else { self.primitive_color };
                    self.rasterize_triangle(edge, base_color, shade, coords, rdram);
                    self.current = self.current.wrapping_add((words - 1) * 8);
                }
                0x3F => {
                    self.color_format = ((command >> 53) & 7) as u8;
                    self.color_size = ((command >> 51) & 3) as u8;
                    self.color_width = ((command >> 32) & 0x3FF) as u32;
                    self.color_address = (command & 0x03FF_FFFF) as u32;
                }
                _ => {}
            }
            self.current = self.current.wrapping_add(8);
        }
    }

    fn rasterize_triangle(
        &mut self,
        edge: [u64; 4],
        color: u32,
        shade: u32,
        coords: Option<TexCoords>,
        rdram: &mut [u8],
    ) {
        if self.color_size != 2 || self.color_format != 2 {
            return;
        }
        let Some(setup) = EdgeSetup::parse(edge) else {
            return;
        };
        // The RDP requires all four subpixel rows of a scanline to be between
        // Yh and Yl; a partially covered top/bottom line is dropped.
        let y_start = (setup.yh + 3) / 4;
        let y_end = (setup.yl - 1) / 4;
        if y_end < y_start {
            return;
        }
        let origin_y = setup.yh.div_euclid(4);
        let origin_x = [setup.xh, setup.xm, setup.xl]
            .iter()
            .map(|value| (value >> 16) as i32)
            .min()
            .unwrap_or(0);

        let modes = self.other_modes;
        for y in y_start.max(0)..=y_end {
            let mut spans = [(0i64, 0i64); 4];
            let mut any = false;
            for (sub, span) in spans.iter_mut().enumerate() {
                *span = setup.span(y * 4 + sub as i32);
                any |= span.1 > span.0;
            }
            if !any {
                continue;
            }
            for x in 0..self.color_width as i32 {
                let center = (i64::from(x) << 16) + 0x8000;
                let coverage = spans
                    .iter()
                    .filter(|(left, right)| center >= *left && center < *right)
                    .count() as u8
                    * 2;
                if coverage == 0 {
                    continue;
                }
                self.render_pixel(
                    x,
                    y,
                    coverage,
                    color,
                    shade,
                    coords,
                    (origin_x, origin_y),
                    modes,
                    rdram,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_pixel(
        &self,
        x: i32,
        y: i32,
        coverage: u8,
        color: u32,
        shade: u32,
        coords: Option<TexCoords>,
        origin: (i32, i32),
        modes: OtherModes,
        rdram: &mut [u8],
    ) {
        let texel = coords
            .and_then(|coords| self.sample_texture_coords(&coords, x, y, origin, rdram))
            .unwrap_or(color);
        let combined = if self.combine_mode != 0 {
            combine(
                self.combine_mode,
                shade,
                texel,
                self.primitive_color,
                self.environment_color,
            )
        } else if coords.is_some() {
            texel
        } else {
            color
        };
        let address =
            self.color_address as usize + ((y as u32 * self.color_width + x as u32) as usize) * 2;
        let destination = rdram
            .get(address..address + 2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            .unwrap_or(0);
        let memory = rgba5551_to_8888(destination);
        let mut surface = if modes.cycle == 2 {
            // Copy cycle: the texel (or primitive colour) bypasses the
            // combiner and blender.
            if coords.is_some() {
                texel
            } else {
                color
            }
        } else {
            let cycle = usize::from(modes.cycle == 1);
            let mut pixel = combined;
            if modes.cycle == 1 {
                pixel = blender(
                    modes.blend[0],
                    pixel,
                    memory,
                    self.fog_color,
                    self.blend_color,
                    (shade & 0xFF) as u8,
                    modes.force_blend,
                    false,
                    modes.color_on_coverage,
                    false,
                );
            }
            blender(
                modes.blend[cycle],
                pixel,
                memory,
                self.fog_color,
                self.blend_color,
                (shade & 0xFF) as u8,
                modes.force_blend,
                true,
                modes.color_on_coverage,
                false,
            )
        };
        if modes.dither >> 2 < 2 {
            surface = dither(surface, x, y, ((modes.dither >> 2) & 3) as usize);
        }
        if modes.aa {
            let coverage8 = u32::from(coverage).min(8);
            let mut alpha = if modes.alpha_cvg_select {
                coverage8 * 255 / 8
            } else if modes.cvg_times_alpha {
                (surface & 0xFF) * coverage8 / 8
            } else {
                surface & 0xFF
            };
            let memory_coverage = ((memory & 0xFF) >> 5) as u8;
            let stored = coverage_blend(coverage.min(7), memory_coverage, modes.coverage_mode);
            alpha = (alpha & !0xE0) | (u32::from(stored) << 5);
            surface = (surface & 0xFFFF_FF00) | alpha.min(255);
        }
        let pixel = rgba8888_to_5551(surface).to_be_bytes();
        if let Some(slot) = rdram.get_mut(address..address + 2) {
            slot.copy_from_slice(&pixel);
        }
    }

    fn sample_texture_coords(
        &self,
        coords: &TexCoords,
        x: i32,
        y: i32,
        origin: (i32, i32),
        rdram: &[u8],
    ) -> Option<u32> {
        let tile = self.tiles[0];
        let size = self.tile_sizes[0];
        if size.s_high <= size.s_low || size.t_high <= size.t_low {
            return None;
        }
        let (min_x, min_y) = origin;
        let scale = |value: i32| i64::from(value);
        let mut s_fixed = scale(coords.s[0])
            + i64::from(x - min_x) * scale(coords.s[1])
            + i64::from(y - min_y) * scale(coords.s[2]);
        let mut t_fixed = scale(coords.t[0])
            + i64::from(x - min_x) * scale(coords.t[1])
            + i64::from(y - min_y) * scale(coords.t[2]);
        let w_fixed = scale(coords.w[0])
            + i64::from(x - min_x) * scale(coords.w[1])
            + i64::from(y - min_y) * scale(coords.w[2]);
        if w_fixed != 0 && (coords.w[0] | coords.w[1] | coords.w[2]) != 0 {
            s_fixed = (s_fixed * 65_536) / w_fixed;
            t_fixed = (t_fixed * 65_536) / w_fixed;
        }
        let mut s = (s_fixed >> 16) as i32;
        let mut t = (t_fixed >> 16) as i32;
        s >>= tile.s_shift;
        t >>= tile.t_shift;
        let s = tile_coordinate(
            s as f32,
            size.s_low as i32,
            size.s_high as i32,
            tile.s_clamp,
            tile.s_mirror,
            tile.s_mask,
        );
        let t = tile_coordinate(
            t as f32,
            size.t_low as i32,
            size.t_high as i32,
            tile.t_clamp,
            tile.t_mirror,
            tile.t_mask,
        );
        let row_bytes = usize::from(tile.line) * 8;
        let base = self.texture.address as usize + usize::from(tile.address) * 8;
        let sample = match tile.size {
            0 => {
                let offset = base + t.max(0) as usize * row_bytes + (s.max(0) as usize) / 2;
                let byte = *rdram.get(offset)?;
                let nibble = if s & 1 == 0 { byte >> 4 } else { byte & 0xF };
                if tile.format == 2 && self.tlut_valid {
                    rgba5551(self.tlut[usize::from(nibble)])
                } else {
                    intensity(nibble)
                }
            }
            1 => {
                let offset = base + t.max(0) as usize * row_bytes + s.max(0) as usize;
                let byte = *rdram.get(offset)?;
                if tile.format == 2 && self.tlut_valid {
                    rgba5551(self.tlut[usize::from(byte)])
                } else {
                    intensity(byte)
                }
            }
            2 => {
                let offset = base + t.max(0) as usize * row_bytes + s.max(0) as usize * 2;
                let bytes = rdram.get(offset..offset + 2)?;
                let texel = u16::from_be_bytes([bytes[0], bytes[1]]);
                match tile.format {
                    0 => rgba5551(texel),
                    3 => {
                        let i = ((texel >> 8) & 0xFF) as u8;
                        let a = (texel & 0xFF) as u8;
                        u32::from(i) << 24 | u32::from(i) << 16 | u32::from(i) << 8 | u32::from(a)
                    }
                    _ => {
                        let i = ((texel >> 8) & 0xFF) as u8;
                        u32::from(i) << 24 | u32::from(i) << 16 | u32::from(i) << 8 | 0xFF
                    }
                }
            }
            _ => {
                let offset = base + t.max(0) as usize * row_bytes + s.max(0) as usize * 2;
                let bytes = rdram.get(offset..offset + 2)?;
                rgba5551(u16::from_be_bytes([bytes[0], bytes[1]]))
            }
        };
        Some(sample)
    }

    fn fill_rectangle(&mut self, command: u64, rdram: &mut [u8]) {
        let x_low = ((command >> 44) & 0xFFF) as u32;
        let y_low = ((command >> 32) & 0xFFF) as u32;
        let x_high = ((command >> 12) & 0xFFF) as u32;
        let y_high = (command & 0xFFF) as u32;
        if self.color_size != 2 || self.color_format != 2 {
            return;
        }
        let color = rgba8888_to_5551(self.fill_color);
        let bytes = color.to_be_bytes();
        let x_start = x_low.min(x_high);
        let x_end = x_low.max(x_high);
        let y_start = y_low.min(y_high);
        let y_end = y_low.max(y_high);
        for y in y_start..=y_end {
            for x in x_start..=x_end {
                if self.color_width != 0 && x >= self.color_width {
                    continue;
                }
                let pixel = self.color_address as usize + ((y * self.color_width + x) as usize) * 2;
                if let Some(slot) = rdram.get_mut(pixel..pixel + 2) {
                    slot.copy_from_slice(&bytes);
                }
            }
        }
    }
}

fn sext14(value: u64) -> i32 {
    let value = (value & 0x3FFF) as i32;
    if value & 0x2000 != 0 {
        value - 0x4000
    } else {
        value
    }
}

fn fixed16_i64(integer: u64, fraction: u64) -> i64 {
    i64::from(integer as u16 as i16) * 65_536 + i64::from(fraction as u16)
}

/// Parsed triangle edge coefficients. Y is in 1/4-pixel units and X in 16.16
/// pixels; each of the three edge points carries its slope along the edge.
struct EdgeSetup {
    lmajor: bool,
    yl: i32,
    ym: i32,
    yh: i32,
    xl: i64,
    xh: i64,
    xm: i64,
    dxldy: i64,
    dxhdy: i64,
    dxmdy: i64,
}

impl EdgeSetup {
    fn parse(edge: [u64; 4]) -> Option<Self> {
        let setup = Self {
            lmajor: edge[0] & (1 << 55) != 0,
            yl: sext14(edge[0] >> 32),
            ym: sext14(edge[0] >> 16),
            yh: sext14(edge[0]),
            xl: fixed16_i64(edge[1] >> 48, edge[1] >> 32),
            dxldy: fixed16_i64(edge[1] >> 16, edge[1]),
            xh: fixed16_i64(edge[2] >> 48, edge[2] >> 32),
            dxhdy: fixed16_i64(edge[2] >> 16, edge[2]),
            xm: fixed16_i64(edge[3] >> 48, edge[3] >> 32),
            dxmdy: fixed16_i64(edge[3] >> 16, edge[3]),
        };
        (setup.yl > setup.yh).then_some(setup)
    }

    /// Left/right edge X at a subpixel row, in 16.16 pixels.
    fn span(&self, row: i32) -> (i64, i64) {
        // Interpolation bases: the top edge is latched at the start of its
        // scanline (Yh rounded down to a whole scanline), the lower at Ym.
        let yh_base = self.yh & !3;
        let xh = self.xh + i64::from(row - yh_base) * self.dxhdy / 4;
        let xm = self.xm + i64::from(row - yh_base) * self.dxmdy / 4;
        let mut xl = self.xl + i64::from(row - self.ym) * self.dxldy / 4;
        if row < self.ym {
            xl = xm;
        }
        if self.lmajor {
            (xh, xl)
        } else {
            (xl, xh)
        }
    }
}

fn read_word(rdram: &[u8], offset: usize) -> Option<u64> {
    let bytes = rdram.get(offset..offset + 8)?;
    let mut word = [0u8; 8];
    word.copy_from_slice(bytes);
    Some(u64::from_be_bytes(word))
}

fn shade_color(word: u64) -> u32 {
    let r = ((word >> 48) & 0xFFFF) >> 8;
    let g = ((word >> 32) & 0xFFFF) >> 8;
    let b = ((word >> 16) & 0xFFFF) >> 8;
    let a = (word & 0xFFFF) >> 8;
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32
}

fn fixed16_word(integer: u64, fraction: u64) -> i32 {
    ((integer as u16 as i16 as i32) << 16) | fraction as u16 as i32
}

fn parse_texcoords(rdram: &[u8], offset: usize) -> TexCoords {
    let word = |index: usize| read_word(rdram, offset + index * 8).unwrap_or(0);
    let coord = |shift: u64| {
        [
            fixed16_word(word(0) >> shift, word(2) >> shift),
            fixed16_word(word(1) >> shift, word(3) >> shift),
            fixed16_word(word(5) >> shift, word(7) >> shift),
            fixed16_word(word(4) >> shift, word(6) >> shift),
        ]
    };
    TexCoords {
        s: coord(48),
        t: coord(32),
        w: coord(16),
    }
}

fn channel(color: u32, channel: usize) -> i32 {
    ((color >> (24 - channel * 8)) & 0xFF) as i32
}

fn combine_input(
    selector: u64,
    channel_index: usize,
    shade: u32,
    texel: u32,
    primitive: u32,
    environment: u32,
) -> i32 {
    match selector & 0x7 {
        0 => 0,
        1 => 255,
        2 => channel(shade, channel_index),
        3 => channel(texel, channel_index),
        4 => channel(primitive, channel_index),
        5 => channel(environment, channel_index),
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn combine_cycle(
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    shade: u32,
    texel: u32,
    primitive: u32,
    environment: u32,
) -> u32 {
    let mut result = 0u32;
    for channel_index in 0..4 {
        let av = combine_input(a, channel_index, shade, texel, primitive, environment);
        let bv = combine_input(b, channel_index, shade, texel, primitive, environment);
        let cv = combine_input(c, channel_index, shade, texel, primitive, environment);
        let dv = combine_input(d, channel_index, shade, texel, primitive, environment);
        let value = (((av - bv) * cv) >> 8) + dv;
        let value = value.clamp(0, 255) as u32;
        result |= value << (24 - channel_index * 8);
    }
    result
}

fn combine(mode: u64, shade: u32, texel: u32, primitive: u32, environment: u32) -> u32 {
    let cycle0 = combine_cycle(
        (mode >> 52) & 0xF,
        (mode >> 28) & 0xF,
        (mode >> 47) & 0x1F,
        (mode >> 15) & 0x7,
        shade,
        texel,
        primitive,
        environment,
    );
    let cycle1 = combine_cycle(
        (mode >> 37) & 0xF,
        (mode >> 24) & 0xF,
        (mode >> 32) & 0x1F,
        (mode >> 6) & 0x7,
        cycle0,
        texel,
        primitive,
        environment,
    );
    let alpha0 = combine_cycle(
        (mode >> 44) & 0x7,
        (mode >> 12) & 0x7,
        (mode >> 41) & 0x7,
        (mode >> 9) & 0x7,
        shade,
        texel,
        primitive,
        environment,
    );
    let alpha1 = combine_cycle(
        (mode >> 21) & 0x7,
        (mode >> 3) & 0x7,
        (mode >> 18) & 0x7,
        mode & 0x7,
        alpha0,
        texel,
        primitive,
        environment,
    );
    (cycle1 & 0xFFFF_FF00) | (alpha1 & 0xFF)
}

fn blend_source(selector: u8, pixel: u32, memory: u32, blend_color: u32, fog_color: u32) -> u32 {
    match selector {
        0 => pixel,
        1 => memory,
        2 => blend_color,
        _ => fog_color,
    }
}

fn blend_alpha(selector: u8, pixel: u32, fog_color: u32, shade_alpha: u8) -> u8 {
    match selector {
        0 => (pixel & 0xFF) as u8,
        1 => (fog_color & 0xFF) as u8,
        2 => shade_alpha,
        _ => 0,
    }
}

fn blend_factor(selector: u8, alpha: u8, memory: u32) -> u8 {
    match selector {
        0 => !alpha,
        1 => (memory & 0xFF) as u8,
        2 => 0xFF,
        _ => 0,
    }
}

/// RDP blender: `(rgb0 * a0 + rgb1 * (a1 + 1)) >> 5`.
#[allow(clippy::too_many_arguments)]
fn blender(
    modes: [u8; 4],
    pixel: u32,
    memory: u32,
    fog_color: u32,
    blend_color: u32,
    shade_alpha: u8,
    _force_blend: bool,
    final_cycle: bool,
    color_on_coverage: bool,
    coverage_wrap: bool,
) -> u32 {
    let rgb1 = blend_source(modes[2], pixel, memory, blend_color, fog_color);
    if final_cycle && color_on_coverage && !coverage_wrap {
        return rgb1;
    }
    let rgb0 = blend_source(modes[0], pixel, memory, blend_color, fog_color);
    if final_cycle && modes[1] == 0 && modes[3] == 0 && (pixel & 0xFF) == 0xFF {
        return rgb0;
    }
    let a0 = blend_alpha(modes[1], pixel, fog_color, shade_alpha);
    let a1 = blend_factor(modes[3], a0, memory);
    let a0 = u32::from(a0 >> 3);
    let a1 = u32::from(a1 >> 3);
    let mut result = 0u32;
    for channel_index in 0..3 {
        let c0 = (rgb0 >> (24 - channel_index * 8)) & 0xFF;
        let c1 = (rgb1 >> (24 - channel_index * 8)) & 0xFF;
        let value = (c0 * a0 + c1 * (a1 + 1)) >> 5;
        result |= value.min(255) << (24 - channel_index * 8);
    }
    (result & 0xFFFF_FF00) | (pixel & 0xFF)
}

fn dither(color: u32, x: i32, y: i32, mode: usize) -> u32 {
    const MATRICES: [[u8; 16]; 2] = [
        [0, 6, 1, 7, 4, 2, 5, 3, 3, 5, 2, 4, 7, 1, 6, 0],
        [0, 4, 1, 5, 4, 0, 5, 1, 3, 7, 2, 6, 7, 3, 6, 2],
    ];
    let threshold = u32::from(MATRICES[mode][(y as usize & 3) * 4 + (x as usize & 3)]);
    let mut result = color & 0xFF;
    for channel_index in 0..3 {
        let value = (color >> (24 - channel_index * 8)) & 0xFF;
        let quantized = (value & 0xF8) + if value & 7 > threshold { 8 } else { 0 };
        result |= quantized.min(0xFF) << (24 - channel_index * 8);
    }
    result | (color & 0xFF)
}

fn coverage_blend(coverage: u8, memory_coverage: u8, mode: u8) -> u8 {
    match mode {
        0 => (coverage - 1) & 7,
        1 => (coverage + memory_coverage) & 7,
        2 => 7,
        _ => memory_coverage,
    }
}

fn rgba5551_to_8888(color: u16) -> u32 {
    rgba5551(color)
}

fn tile_coordinate(value: f32, low: i32, high: i32, clamp: bool, mirror: bool, mask: u8) -> i32 {
    let mut coordinate = value.round() as i32;
    if mask != 0 {
        let mask = (1i32 << mask) - 1;
        coordinate &= mask;
    }
    if mirror {
        let span = (high - low).max(1);
        let folded = (coordinate - low).rem_euclid(span * 2);
        coordinate = low
            + if folded >= span {
                span * 2 - 1 - folded
            } else {
                folded
            };
    }
    if clamp {
        coordinate = coordinate.clamp(low, high);
    }
    coordinate
}

fn intensity(value: u8) -> u32 {
    u32::from(value) << 24 | u32::from(value) << 16 | u32::from(value) << 8 | 0xFF
}

fn rgba5551(color: u16) -> u32 {
    let r = expand5((color >> 11) & 0x1F);
    let g = expand5((color >> 6) & 0x1F);
    let b = expand5((color >> 1) & 0x1F);
    let a = if color & 1 != 0 { 0xFF } else { 0 };
    u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | (a as u32)
}

fn expand5(value: u16) -> u8 {
    let value = value as u8;
    (value << 3) | (value >> 2)
}

fn rgba8888_to_5551(color: u32) -> u16 {
    let r = ((color >> 24) & 0xFF) as u16 >> 3;
    let g = ((color >> 16) & 0xFF) as u16 >> 3;
    let b = ((color >> 8) & 0xFF) as u16 >> 3;
    let a = ((color & 0xFF) as u16 >> 7) & 1;
    (r << 11) | (g << 6) | (b << 1) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_command(rdram: &mut [u8], index: usize, command: u64) {
        let offset = index * 8;
        rdram[offset..offset + 8].copy_from_slice(&command.to_be_bytes());
    }

    #[test]
    fn parses_combiner_state_and_triangle_length() {
        let mut rdram = vec![0u8; 0x4000];
        put_command(&mut rdram, 0, (0x3Au64 << 56) | 0x1122_33FF);
        put_command(&mut rdram, 1, (0x3Bu64 << 56) | 0x4455_66FF);
        put_command(&mut rdram, 2, (0x3Cu64 << 56) | 0x00AA_55AA);
        put_command(&mut rdram, 3, 0x08u64 << 56); // triangle + 3 edge words
        put_command(&mut rdram, 4, 0);
        put_command(&mut rdram, 5, 0);
        put_command(&mut rdram, 6, 0);
        put_command(&mut rdram, 7, (0x37u64 << 56) | 0x0000_00FF);
        let mut rdp = Rdp::new();
        rdp.write_register(0x00, 0, &mut rdram);
        rdp.write_register(0x04, 64, &mut rdram);
        assert_eq!(rdp.primitive_color(), 0x1122_33FF);
        assert_eq!(rdp.environment_color(), 0x4455_66FF);
        assert_eq!(rdp.combine_mode(), (0x3Cu64 << 56) | 0x00AA_55AA);
        assert_eq!(rdp.triangles(), 1);
        // The following SET_FILL_COLOR was reached (stream stayed aligned).
        assert_eq!(rdp.fill_color, 0x0000_00FF);
    }

    #[test]
    fn dither_pattern_rounds_rgb_up_by_threshold() {
        // 0x85 has a fractional 5-bit remainder of 5; the magic matrix entry
        // at (0,0) is 0 so it rounds up, while (1,0) is 6 and stays.
        assert_eq!(dither(0x8585_85FF, 0, 0, 0), 0x8888_88FF);
        assert_eq!(dither(0x8585_85FF, 1, 0, 0), 0x8080_80FF);
    }

    #[test]
    fn fills_a_solid_rectangle() {
        let mut rdram = vec![0u8; 0x4000];
        let color_image = (0x3Fu64 << 56) | (2u64 << 53) | (2u64 << 51) | (64u64 << 32) | 0x1000;
        let fill_color = (0x37u64 << 56) | 0xFF00_00FF; // red, opaque
        let rectangle = (0x36u64 << 56) | (3u64 << 12) | 3;
        put_command(&mut rdram, 0, color_image);
        put_command(&mut rdram, 1, fill_color);
        put_command(&mut rdram, 2, rectangle);

        let mut rdp = Rdp::new();
        rdp.write_register(0x00, 0, &mut rdram);
        rdp.write_register(0x04, 24, &mut rdram);

        assert_eq!(&rdram[0x1000..0x1002], &[0xF8, 0x01]);
        // (3,3) is the last filled pixel; (4,0) is outside the rectangle.
        let corner = 0x1000 + (3 * 64 + 3) * 2;
        assert_eq!(&rdram[corner..corner + 2], &[0xF8, 0x01]);
        let outside = 0x1000 + 4 * 2;
        assert_eq!(&rdram[outside..outside + 2], &[0, 0]);
    }
}
