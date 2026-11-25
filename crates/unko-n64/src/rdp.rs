//! RDP scaffold: the DPC register block and a command interpreter for the
//! solid-fill path.
//!
//! Commands are fetched as big-endian 64-bit words from RDRAM between
//! `DPC_START` and `DPC_END`. Implemented: `SET_COLOR_IMAGE`,
//! `SET_FILL_COLOR`, `FILL_RECTANGLE` and `SYNC_FULL`. Triangles, texturing,
//! combiner, blender and depth are later milestones, so this renders solid
//! rectangles into a 16-bit framebuffer.

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
}

/// Tile bounds (`SET_TILE_SIZE`).
#[derive(Debug, Clone, Copy, Default)]
struct TileSize {
    s_low: u16,
    t_low: u16,
    s_high: u16,
    t_high: u16,
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
    combine_mode: u32,
    triangles: u64,
    texture: TextureImage,
    tiles: [Tile; 8],
    tile_sizes: [TileSize; 8],
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
            triangles: 0,
            texture: TextureImage::default(),
            tiles: [Tile::default(); 8],
            tile_sizes: [TileSize::default(); 8],
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
    pub fn combine_mode(&self) -> u32 {
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
                    };
                }
                0x3A => self.primitive_color = command as u32,
                0x3B => self.environment_color = command as u32,
                0x3C => self.combine_mode = command as u32,
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
                        16
                    } else if shaded {
                        12
                    } else if textured {
                        8
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
                    let mut color = self.primitive_color;
                    if shaded {
                        let offset = start + 4 * 8;
                        if let Some(bytes) = rdram.get(offset..offset + 8) {
                            let mut word = [0u8; 8];
                            word.copy_from_slice(bytes);
                            let shade = u64::from_be_bytes(word);
                            let r = ((shade >> 48) & 0xFFFF) >> 8;
                            let g = ((shade >> 32) & 0xFFFF) >> 8;
                            let b = ((shade >> 16) & 0xFFFF) >> 8;
                            let a = (shade & 0xFFFF) >> 8;
                            color = ((r as u32) << 24)
                                | ((g as u32) << 16)
                                | ((b as u32) << 8)
                                | a as u32;
                        }
                    }
                    self.rasterize_triangle(edge, color, textured, rdram);
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

    fn rasterize_triangle(&mut self, edge: [u64; 4], color: u32, textured: bool, rdram: &mut [u8]) {
        if self.color_size != 2 || self.color_format != 2 {
            return;
        }
        let y_low = sext14(edge[0] >> 32) as f32 / 4.0;
        let y_mid = sext14(edge[0] >> 16) as f32 / 4.0;
        let y_high = sext14(edge[0]) as f32 / 4.0;
        let x_low = fixed16(edge[1] >> 48, edge[1] >> 32);
        let x_high = fixed16(edge[2] >> 48, edge[2] >> 32);
        let x_mid = fixed16(edge[3] >> 48, edge[3] >> 32);
        // The edge coefficients identify three points on the triangle's edge
        // functions. This scaffold treats them as the triangle's vertices and
        // rasterises with a half-space test; full RDP edgewalk traversal is a
        // later milestone.
        let vertices = [(x_low, y_low), (x_mid, y_mid), (x_high, y_high)];
        let min_x = vertices
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as i32;
        let max_x = vertices
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i32;
        let min_y = vertices
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::INFINITY, f32::min)
            .ceil()
            .max(0.0) as i32;
        let max_y = vertices
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::NEG_INFINITY, f32::max)
            .floor() as i32;
        let area = edge_function(vertices[0], vertices[1], vertices[2]);
        if area == 0.0 {
            return;
        }
        let bounds = (min_x, min_y, max_x, max_y);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let point = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge_function(vertices[1], vertices[2], point);
                let w1 = edge_function(vertices[2], vertices[0], point);
                let w2 = edge_function(vertices[0], vertices[1], point);
                let inside = if area > 0.0 {
                    w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                } else {
                    w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                };
                if !inside {
                    continue;
                }
                if self.color_width != 0 && x as u32 >= self.color_width {
                    continue;
                }
                let surface = if textured {
                    self.sample_texture(x, y, bounds, rdram).unwrap_or(color)
                } else {
                    color
                };
                let pixel = rgba8888_to_5551(surface).to_be_bytes();
                let address = self.color_address as usize
                    + ((y as u32 * self.color_width + x as u32) as usize) * 2;
                if let Some(slot) = rdram.get_mut(address..address + 2) {
                    slot.copy_from_slice(&pixel);
                }
            }
        }
    }

    fn sample_texture(
        &self,
        x: i32,
        y: i32,
        bounds: (i32, i32, i32, i32),
        rdram: &[u8],
    ) -> Option<u32> {
        let tile = self.tiles[0];
        let size = self.tile_sizes[0];
        if size.s_high <= size.s_low || size.t_high <= size.t_low {
            return None;
        }
        let (min_x, min_y, max_x, max_y) = bounds;
        let span_x = (max_x - min_x).max(1) as f32;
        let span_y = (max_y - min_y).max(1) as f32;
        let width = f32::from(size.s_high - size.s_low) + 1.0;
        let height = f32::from(size.t_high - size.t_low) + 1.0;
        let s = f32::from(size.s_low) + (x - min_x) as f32 / span_x * (width - 1.0);
        let t = f32::from(size.t_low) + (y - min_y) as f32 / span_y * (height - 1.0);
        let s = tile_coordinate(
            s,
            size.s_low as i32,
            size.s_high as i32,
            tile.s_clamp,
            tile.s_mirror,
            tile.s_mask,
        );
        let t = tile_coordinate(
            t,
            size.t_low as i32,
            size.t_high as i32,
            tile.t_clamp,
            tile.t_mirror,
            tile.t_mask,
        );
        let row_bytes = usize::from(tile.line) * 8;
        let base = self.texture.address as usize + usize::from(tile.address) * 8;
        let texel = match tile.size {
            0 => {
                let offset = base + t.max(0) as usize * row_bytes + (s.max(0) as usize) / 2;
                let byte = *rdram.get(offset)?;
                let nibble = if s & 1 == 0 { byte >> 4 } else { byte & 0xF };
                intensity(nibble)
            }
            1 => {
                let offset = base + t.max(0) as usize * row_bytes + s.max(0) as usize;
                let byte = *rdram.get(offset)?;
                intensity(byte)
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
                let texel = u16::from_be_bytes([bytes[0], bytes[1]]);
                rgba5551(texel)
            }
        };
        Some(texel)
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

fn fixed16(integer: u64, fraction: u64) -> f32 {
    (integer as u16 as i16 as f32) + (fraction as u16 as f32) / 65_536.0
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

fn edge_function(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
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
        assert_eq!(rdp.combine_mode(), 0x00AA_55AA);
        assert_eq!(rdp.triangles(), 1);
        // The following SET_FILL_COLOR was reached (stream stayed aligned).
        assert_eq!(rdp.fill_color, 0x0000_00FF);
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
