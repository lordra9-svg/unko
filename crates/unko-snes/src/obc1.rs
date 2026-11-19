//! OBC1 coprocessor (Metal Combat: Falcon's Revenge).
//!
//! The OBC1 is a small sprite/bitmap helper with an 8 KiB RAM that also serves
//! as the cartridge's battery-backed save. Its registers live in the last
//! sixteen bytes of that RAM window (`$FF0-$FFF`, i.e. `$7FF0-$7FFF` when the
//! window is mapped at `$6000-$7FFF`). The behaviour follows the ISC-licensed
//! ares `sfc/coprocessor/obc1` source: a base-pointer bit selects one of two
//! 1 KiB object tables, `$1FF0-$1FF3` expose a four-byte object selected by
//! `$1FF6` bits 0-6, and `$1FF4` reads/writes two packed bits inside a
//! per-object attribute byte selected by `$1FF6` bits 0-1.

/// Register/window mask of the OBC1 RAM (8 KiB).
pub const OBC1_RAM_SIZE: usize = 0x2000;

/// OBC1 register state.
///
/// The RAM itself is the cartridge's battery-backed save RAM, so it lives in
/// [`crate::rom::Cartridge`] and is serialized with the rest of the save data.
/// Only the latched pointer registers are owned here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Obc1 {
    /// Object index (`$1FF6` bits 0-6).
    address: u16,
    /// Object-table base, `$1800` or `$1C00` (`$1FF5` bit 0).
    baseptr: u16,
    /// Bit position of the packed pair in the attribute byte (`$1FF6` bits 0-1).
    shift: u16,
}

impl Default for Obc1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Obc1 {
    /// Create the coprocessor in its power-on state.
    pub const fn new() -> Self {
        Self {
            address: 0,
            baseptr: 0x1C00,
            shift: 0,
        }
    }

    /// ares `OBC1::power`: latch the base pointer and object index from RAM.
    pub fn power(&mut self, ram: &[u8]) {
        let at = |index: usize| ram.get(index).copied().unwrap_or(0xFF);
        self.baseptr = if at(0x1FF5) & 1 != 0 { 0x1800 } else { 0x1C00 };
        self.address = u16::from(at(0x1FF6) & 0x7F);
        self.shift = u16::from(at(0x1FF6) & 3) << 1;
    }

    /// Read from the OBC1 window. `address` is already masked to 13 bits.
    pub fn read(&self, ram: &[u8], address: u32) -> u8 {
        let address = (address & 0x1FFF) as u16;
        let ram_read = |index: u16| ram.get(index as usize).copied().unwrap_or(0xFF);
        match address {
            0x1FF0 => ram_read(self.baseptr + (self.address << 2)),
            0x1FF1 => ram_read(self.baseptr + (self.address << 2) + 1),
            0x1FF2 => ram_read(self.baseptr + (self.address << 2) + 2),
            0x1FF3 => ram_read(self.baseptr + (self.address << 2) + 3),
            0x1FF4 => ram_read(self.baseptr + (self.address >> 2) + 0x200),
            _ => ram_read(address),
        }
    }

    /// Write to the OBC1 window. `address` is already masked to 13 bits.
    pub fn write(&mut self, ram: &mut [u8], address: u32, data: u8) {
        let address = (address & 0x1FFF) as u16;
        let ram_write = |ram: &mut [u8], index: u16, data: u8| {
            if let Some(byte) = ram.get_mut(index as usize) {
                *byte = data;
            }
        };
        match address {
            0x1FF0..=0x1FF3 => {
                let index = self.baseptr + (self.address << 2) + (address - 0x1FF0);
                ram_write(ram, index, data);
            }
            0x1FF4 => {
                let index = self.baseptr + (self.address >> 2) + 0x200;
                let mut value = ram.get(index as usize).copied().unwrap_or(0xFF);
                value = (value & !(3u8 << self.shift)) | ((data & 3) << self.shift);
                ram_write(ram, index, value);
            }
            0x1FF5 => {
                self.baseptr = if data & 1 != 0 { 0x1800 } else { 0x1C00 };
                ram_write(ram, address, data);
            }
            0x1FF6 => {
                self.address = u16::from(data & 0x7F);
                self.shift = u16::from(data & 3) << 1;
                ram_write(ram, address, data);
            }
            _ => ram_write(ram, address, data),
        }
    }

    /// Whether `address` falls in an OBC1 RAM window.
    ///
    /// ares `OBC1-LOROM-RAM` maps the 8 KiB RAM (with the register aliases at
    /// `$1FF0-$1FF7`) at `$00-$3F/$80-$BF:$6000-$7FFF` and at
    /// `$70-$71/$F0-$F1:$6000-$7FFF` plus `$E000-$FFFF`.
    pub const fn window(address: u32) -> bool {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = address & 0xFFFF;
        let bank7 = bank & 0x7F;
        if bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF) {
            offset >= 0x6000 && offset <= 0x7FFF
        } else if bank7 == 0x70 || bank7 == 0x71 {
            (offset >= 0x6000 && offset <= 0x7FFF) || offset >= 0xE000
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_match_the_ares_board_map() {
        for address in [0x00_6000, 0x00_7FFF, 0x3F_6000, 0x80_7000, 0xBF_7FFF] {
            assert!(Obc1::window(address), "{address:06X}");
        }
        for address in [
            0x70_6000, 0x71_7FFF, 0x70_E000, 0x71_FFFF, 0xF0_6000, 0xF1_E000,
        ] {
            assert!(Obc1::window(address), "{address:06X}");
        }
        for address in [
            0x00_5FFF, 0x00_8000, 0x40_6000, 0x70_5FFF, 0x70_DFFF, 0x72_E000,
        ] {
            assert!(!Obc1::window(address), "{address:06X}");
        }
    }

    #[test]
    fn power_latches_the_table_and_object_index() {
        let mut ram = vec![0u8; OBC1_RAM_SIZE];
        ram[0x1FF5] = 1;
        ram[0x1FF6] = 0x85;
        let mut obc1 = Obc1::new();
        obc1.power(&ram);
        assert_eq!(obc1.baseptr, 0x1800);
        assert_eq!(obc1.address, 0x05);
        assert_eq!(obc1.shift, 2);
    }

    #[test]
    fn object_bytes_and_attribute_pair_round_trip() {
        let mut ram = vec![0u8; OBC1_RAM_SIZE];
        let mut obc1 = Obc1::new();
        obc1.power(&ram);
        // Select object 5 and the first attribute-bit pair.
        obc1.write(&mut ram, 0x1FF6, 0x05);
        let base = 0x1C00 + (5 << 2);
        for (i, value) in [0x11, 0x22, 0x33, 0x44].into_iter().enumerate() {
            obc1.write(&mut ram, 0x1FF0 + i as u32, value);
        }
        for (i, value) in [0x11, 0x22, 0x33, 0x44].into_iter().enumerate() {
            assert_eq!(obc1.read(&ram, 0x1FF0 + i as u32), value);
        }
        assert_eq!(&ram[base..base + 4], &[0x11, 0x22, 0x33, 0x44]);

        // `$1FF4` writes only the selected two-bit field of the attribute byte.
        let attribute = 0x1E01; // base + (object 5 >> 2) + $200
        ram[attribute] = 0xF0;
        // Keep object 5 while selecting attribute bits 2-3 (`$05` & 3 = 1).
        obc1.write(&mut ram, 0x1FF6, 0x05);
        obc1.write(&mut ram, 0x1FF4, 0x02);
        assert_eq!(ram[attribute], 0xF8);
        assert_eq!(obc1.read(&ram, 0x1FF4), 0xF8);
    }

    #[test]
    fn base_pointer_switches_the_object_table() {
        let mut ram = vec![0u8; OBC1_RAM_SIZE];
        let mut obc1 = Obc1::new();
        obc1.power(&ram);
        obc1.write(&mut ram, 0x1FF6, 0x00);
        obc1.write(&mut ram, 0x1FF0, 0xAA);
        assert_eq!(ram[0x1C00], 0xAA);
        obc1.write(&mut ram, 0x1FF5, 0x01);
        obc1.write(&mut ram, 0x1FF0, 0x55);
        assert_eq!(ram[0x1800], 0x55);
        assert_eq!(ram[0x1C00], 0xAA);
    }
}
