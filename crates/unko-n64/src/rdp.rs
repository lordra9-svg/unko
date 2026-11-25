//! RDP scaffold: the DPC register block and a command interpreter for the
//! solid-fill path.
//!
//! Commands are fetched as big-endian 64-bit words from RDRAM between
//! `DPC_START` and `DPC_END`. Implemented: `SET_COLOR_IMAGE`,
//! `SET_FILL_COLOR`, `FILL_RECTANGLE` and `SYNC_FULL`. Triangles, texturing,
//! combiner, blender and depth are later milestones, so this renders solid
//! rectangles into a 16-bit framebuffer.

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
                0x3A => self.primitive_color = command as u32,
                0x3B => self.environment_color = command as u32,
                0x3C => self.combine_mode = command as u32,
                0x08..=0x0F => {
                    self.triangles += 1;
                    let shaded = opcode >= 0x0C;
                    let words = if shaded { 12 } else { 4 };
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
                    self.rasterize_triangle(edge, color, rdram);
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

    fn rasterize_triangle(&mut self, edge: [u64; 4], color: u32, rdram: &mut [u8]) {
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
        let pixel = rgba8888_to_5551(color).to_be_bytes();
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
                let address = self.color_address as usize
                    + ((y as u32 * self.color_width + x as u32) as usize) * 2;
                if let Some(slot) = rdram.get_mut(address..address + 2) {
                    slot.copy_from_slice(&pixel);
                }
            }
        }
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
