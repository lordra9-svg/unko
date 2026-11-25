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
                    // Triangle: edge coefficients (3 words), plus shade colour
                    // for the shaded variants. Rasterisation is a later
                    // milestone, but the command length is consumed so the
                    // stream stays aligned.
                    self.triangles += 1;
                    let words = if opcode >= 0x0C { 12 } else { 4 };
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
