//! BS-X (Satellaview) cartridge.
//!
//! Reference: bsnes' `BsxCartridge` as documented by ikari_01/nocash on the
//! Super Famicom Development Wiki ("BS-X Memory Map", "BS-X MMIO") and the
//! MiSTer SNES core's `BSXMap`/`MCC` RTL, which derive from the same source.
//!
//! The cartridge contains:
//!
//! * a 1 MiB BIOS ("base cartridge") mapped as LoROM;
//! * 512 KiB of PSRAM (the download buffer), mappable in LoROM or HiROM mode;
//! * 32 KiB of battery SRAM at `$10-$17:$5000-$5FFF`;
//! * sixteen 1-bit MCC registers at `$00-$0F:$5000-$5FFF` (data bit 7);
//! * an optional BS Memory Pack (the "Data Pak") on the extension port.
//!
//! Register `$0e` commits the changes made to `$02-$0d`; until then the
//! previously committed values stay in effect for memory mapping.

/// BIOS size (1 MiB).
pub const BIOS_SIZE: usize = 0x10_0000;
/// PSRAM size (512 KiB).
pub const PSRAM_SIZE: usize = 0x8_0000;
/// Battery SRAM size (32 KiB).
pub const SRAM_SIZE: usize = 0x8000;

/// Which device an address selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BsxBus {
    /// BS-X BIOS at the given offset.
    Bios(u32),
    /// PSRAM at the given offset.
    Psram(u32),
    /// BS Memory Pack at the given offset.
    Pak(u32),
    /// Battery SRAM at the given offset.
    Sram(u32),
    /// MCC register file; the payload is the register index (`$0-$f`).
    Mmio(u8),
    /// Open bus.
    Open,
}

/// A BS-X cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bsx {
    bios: Vec<u8>,
    psram: Vec<u8>,
    sram: Vec<u8>,
    pak: Option<Vec<u8>>,
    /// Values written to each MCC register ("lastwrite" in nocash's notes).
    lastwrite: [bool; 16],
    /// Committed values that actually control the memory map (`$02-$0d`).
    applied: [bool; 16],
    /// Data-Pak ready IRQ flag (`$00`), cleared by writing any value.
    irq_flag: bool,
}

impl Bsx {
    /// Create a BS-X cartridge from the BIOS image.
    pub fn new(bios: Vec<u8>) -> Self {
        // Reset state (`nocash`, "Reset state"): r01=0 r02=1 r03=1 r04=0 r05=1
        // r06=1 r07=1 r08=1 r09=1 r0a=0 r0b=1 r0c=0 r0d=0.
        let mut applied = [false; 16];
        for (index, value) in [
            (1, false),
            (2, true),
            (3, true),
            (4, false),
            (5, true),
            (6, true),
            (7, true),
            (8, true),
            (9, true),
            (10, false),
            (11, true),
            (12, false),
            (13, false),
        ] {
            applied[index] = value;
        }
        Self {
            bios,
            psram: vec![0; PSRAM_SIZE],
            sram: vec![0; SRAM_SIZE],
            pak: None,
            lastwrite: applied,
            applied,
            irq_flag: false,
        }
    }

    /// Attach a BS Memory Pack image.
    pub fn load_pack(&mut self, image: Vec<u8>) {
        self.pak = Some(image);
    }

    /// Whether a BS Memory Pack is present.
    pub fn has_pack(&self) -> bool {
        self.pak.is_some()
    }

    /// BIOS image.
    pub fn bios(&self) -> &[u8] {
        &self.bios
    }

    /// PSRAM contents.
    pub fn psram(&self) -> &[u8] {
        &self.psram
    }

    /// Mutable PSRAM contents.
    pub fn psram_mut(&mut self) -> &mut [u8] {
        &mut self.psram
    }

    /// Battery SRAM.
    pub fn sram(&self) -> &[u8] {
        &self.sram
    }

    /// The committed MCC register bits.
    pub fn applied_registers(&self) -> &[bool; 16] {
        &self.applied
    }

    /// Read one MCC register (data bit 7, as the S-CPU sees it).
    pub fn mmio_read(&self, index: u8) -> u8 {
        let value = match index & 0x0F {
            0x00 => self.irq_flag,
            0x01 => self.lastwrite[1],
            // `$02-$0d` read back the committed bits that drive the map.
            0x02..=0x0D => self.applied[usize::from(index & 0x0F)],
            // `$0e` (commit) and `$0f` (hidden bits) read as zero.
            _ => false,
        };
        u8::from(value) << 7
    }

    /// Write one MCC register (data bit 7).
    pub fn mmio_write(&mut self, index: u8, value: u8) {
        let bit = value & 0x80 != 0;
        let index = index & 0x0F;
        match index {
            0x00 => self.irq_flag = false, // write acknowledges the IRQ
            0x0E => {
                // Commit r02-r0d.
                for slot in 0x02usize..=0x0D {
                    self.applied[slot] = self.lastwrite[slot];
                }
            }
            0x0F => {} // hidden-bit access: no observable effect here
            _ => self.lastwrite[usize::from(index)] = bit,
        }
    }

    /// Whether `address` falls in the BS-X ROM area (ares/bsnes `ROM_AREA`).
    ///
    /// `$00-$3f:$0000-$7fff` and `$7e-$7f` are WRAM/expansion space, not
    /// cartridge space.
    fn is_rom_area(bank: u8, offset: u16) -> bool {
        !(bank == 0x7E || bank == 0x7F || (bank < 0x40 && offset < 0x8000))
    }

    fn bit(&self, index: usize) -> bool {
        self.applied[index]
    }

    fn enabled(&self, slow: usize, fast: usize, bank: u8) -> bool {
        if bank & 0x80 == 0 {
            self.bit(slow)
        } else {
            self.bit(fast)
        }
    }

    /// Decode a 24-bit address.
    pub fn map(&self, address: u32) -> BsxBus {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as u16;

        // MCC register file: $00-$0f:$5000-$5fff (mirrored).
        if bank <= 0x0F && (0x5000..=0x5FFF).contains(&offset) {
            return BsxBus::Mmio(bank);
        }
        // Battery SRAM: $10-$17:$5000-$5fff, eight 4 KiB banks.
        if (0x10..=0x17).contains(&bank) && (0x5000..=0x5FFF).contains(&offset) {
            let slot = u32::from(bank - 0x10);
            return BsxBus::Sram((slot << 12) | (u32::from(offset) & 0x0FFF));
        }

        // Bit slices mirroring the RTL's `CA(23)`, `CA(22)`, ... .
        let ca22 = bank & 0x40 != 0;
        let ca21 = bank & 0x20 != 0;
        let ca20 = bank & 0x10 != 0;

        // Offset formulas from the RTL: `CA(hi..lo) & CA(14 downto 0)`.
        let bios_offset = ((u32::from(bank) & 0x1F) << 15) | (u32::from(offset) & 0x7FFF);
        let lorom_psram = ((u32::from(bank) & 0x0F) << 15) | (u32::from(offset) & 0x7FFF);
        let hirom_psram = (((address >> 15) & 0x0F) << 15) | (u32::from(offset) & 0x7FFF);
        let hirom_mode = self.bit(2);
        let psram_slow = self.bit(3);
        let psram_fast = self.bit(4);
        let psram_enabled = if bank & 0x80 == 0 {
            psram_slow
        } else {
            psram_fast
        };

        // Windows that are always PSRAM and bypass the ROM-area test (the RTL
        // assigns them unconditionally, unlike the priority chain below).
        if hirom_mode {
            // `$20-$3f` / `$a0-$bf:$6000-$7fff`: 8 KiB snippets.
            if (bank >> 5) & 3 == 1 && (offset >> 12) == 6 {
                return BsxBus::Psram(hirom_psram);
            }
        } else if psram_enabled && (bank >> 4) & 7 == 7 && offset < 0x8000 {
            // `$70-$7d` / `$f0-$ff:$0000-$7fff`.
            return BsxBus::Psram(lorom_psram);
        }

        if !Self::is_rom_area(bank, offset) {
            return BsxBus::Open;
        }

        let pak_offset = if hirom_mode {
            hirom_psram
        } else {
            ((u32::from(bank) & 0x1F) << 15) | (u32::from(offset) & 0x7FFF)
        };
        let ext_map = self.bit(11);
        let psram_map = u32::from(self.bit(6)) << 1 | u32::from(self.bit(5));

        if !hirom_mode {
            // LoROM mode.
            if self.enabled(7, 8, bank) && !ca22 {
                BsxBus::Bios(bios_offset)
            } else if self.enabled(3, 4, bank) && (u32::from(bank >> 5) & 3) == psram_map {
                BsxBus::Psram(lorom_psram)
            } else if self.enabled(9, 10, bank) && ca22 == ext_map && !ca21 {
                BsxBus::Open // EXTMEM: no chip installed
            } else if bank >> 4 != 0x7 {
                BsxBus::Pak(pak_offset)
            } else {
                BsxBus::Open
            }
        } else {
            // HiROM mode.
            if self.enabled(7, 8, bank) && !ca22 {
                BsxBus::Bios(bios_offset)
            } else if self.enabled(3, 4, bank) && (u32::from(bank >> 4) & 3) == psram_map && !ca20 {
                BsxBus::Psram(hirom_psram)
            } else if self.enabled(9, 10, bank) && ca21 == ext_map && !ca20 {
                BsxBus::Open // EXTMEM
            } else {
                BsxBus::Pak(pak_offset)
            }
        }
    }

    /// Read a 24-bit address (open bus returns `mdr`).
    pub fn read(&self, address: u32, mdr: u8) -> u8 {
        match self.map(address) {
            BsxBus::Bios(offset) => self.read_bios(offset),
            BsxBus::Psram(offset) => self.psram[(offset as usize) % self.psram.len()],
            BsxBus::Pak(offset) => self
                .pak
                .as_ref()
                .map_or(mdr, |pak| pak[(offset as usize) % pak.len()]),
            BsxBus::Sram(offset) => self.sram[(offset as usize) % self.sram.len()],
            BsxBus::Mmio(index) => self.mmio_read(index),
            BsxBus::Open => mdr,
        }
    }

    fn read_bios(&self, offset: u32) -> u8 {
        if self.bios.is_empty() {
            return 0;
        }
        self.bios[(offset as usize) % self.bios.len()]
    }

    /// Write a 24-bit address. Returns `true` when the cartridge consumed it.
    pub fn write(&mut self, address: u32, value: u8) -> bool {
        match self.map(address) {
            BsxBus::Mmio(index) => {
                self.mmio_write(index, value);
                true
            }
            BsxBus::Sram(offset) => {
                let len = self.sram.len();
                self.sram[(offset as usize) % len] = value;
                true
            }
            BsxBus::Psram(offset) => {
                let len = self.psram.len();
                self.psram[(offset as usize) % len] = value;
                true
            }
            BsxBus::Pak(offset) => {
                // r0c enables Data-Pak writes; r0d is the /WP signal (active low).
                if self.bit(12) && self.bit(13) {
                    if let Some(pak) = self.pak.as_mut() {
                        let len = pak.len();
                        pak[(offset as usize) % len] = value;
                    }
                }
                true
            }
            BsxBus::Bios(_) => false,
            BsxBus::Open => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cartridge() -> Bsx {
        let mut bios = vec![0u8; BIOS_SIZE];
        for (index, byte) in bios.iter_mut().enumerate() {
            *byte = index as u8;
        }
        Bsx::new(bios)
    }

    fn commit(cart: &mut Bsx, index: u8, value: u8) {
        cart.mmio_write(index, value);
        cart.mmio_write(0x0E, 0x80);
    }

    #[test]
    fn power_on_registers_match_nocash() {
        let cart = cartridge();
        let expected = [
            (0x01u8, 0u8),
            (0x02, 0x80),
            (0x03, 0x80),
            (0x04, 0x00),
            (0x05, 0x80),
            (0x06, 0x80),
            (0x07, 0x80),
            (0x08, 0x80),
            (0x09, 0x80),
            (0x0A, 0x00),
            (0x0B, 0x80),
            (0x0C, 0x00),
            (0x0D, 0x00),
        ];
        for (index, value) in expected {
            assert_eq!(cart.mmio_read(index), value, "register ${index:02x}");
        }
        assert_eq!(cart.mmio_read(0x00), 0);
        assert_eq!(cart.mmio_read(0x0E), 0);
        assert_eq!(cart.mmio_read(0x0F), 0);
    }

    #[test]
    fn writes_need_register_0e_to_take_effect() {
        let mut cart = cartridge();
        cart.mmio_write(0x07, 0x00); // request: BIOS off in the slow area
        assert_eq!(cart.mmio_read(0x07), 0x80, "reads show the committed bit");
        // BIOS still mapped at $00:$8000 until the commit.
        assert_eq!(cart.map(0x00_8000), BsxBus::Bios(0));
        cart.mmio_write(0x0E, 0x00); // commit
        assert_eq!(cart.mmio_read(0x07), 0x00);
        // With the BIOS gone and PSRAM at its default HiROM location, the
        // Memory Pack owns $00-$3f:$8000-$ffff.
        assert!(matches!(cart.map(0x00_8000), BsxBus::Pak(_)));
    }

    #[test]
    fn bios_maps_lorom_at_00_and_80() {
        let cart = cartridge();
        assert_eq!(cart.map(0x00_8000), BsxBus::Bios(0));
        assert_eq!(cart.map(0x00_FFFF), BsxBus::Bios(0x7FFF));
        assert_eq!(cart.map(0x01_8000), BsxBus::Bios(0x8000));
        // Fast area is enabled by r08.
        assert_eq!(cart.map(0x80_8000), BsxBus::Bios(0));
    }

    #[test]
    fn psram_lorom_placement_follows_r05_r06() {
        let mut cart = cartridge();
        commit(&mut cart, 0x02, 0x00); // LoROM
        commit(&mut cart, 0x07, 0x00); // BIOS off in the slow area
                                       // Default r05=r06=1 -> banks $60-$6F / $e0-$ef.
        assert_eq!(cart.map(0x60_0000), BsxBus::Psram(0));
        assert_eq!(cart.map(0x61_0000), BsxBus::Psram(0x8000));
        commit(&mut cart, 0x05, 0x00);
        commit(&mut cart, 0x06, 0x00);
        // r05=r06=0 -> banks $00-$0F / $80-$8F (upper half only in LoROM).
        assert_eq!(cart.map(0x00_8000), BsxBus::Psram(0));
        assert_eq!(cart.map(0x01_8000), BsxBus::Psram(0x8000));
    }

    #[test]
    fn psram_lorom_always_maps_70_and_f0_low_half() {
        let mut cart = cartridge();
        commit(&mut cart, 0x02, 0x00); // LoROM
        commit(&mut cart, 0x04, 0x80); // PSRAM on in the fast area
        assert_eq!(cart.map(0x70_0000), BsxBus::Psram(0));
        assert_eq!(cart.map(0x70_7FFF), BsxBus::Psram(0x7FFF));
        assert_eq!(cart.map(0xF0_0000), BsxBus::Psram(0));
    }

    #[test]
    fn hirom_psram_uses_full_banks_and_6000_snippets() {
        let mut cart = cartridge();
        commit(&mut cart, 0x02, 0x80); // HiROM
        commit(&mut cart, 0x07, 0x00); // BIOS off in the slow area
        commit(&mut cart, 0x05, 0x00);
        commit(&mut cart, 0x06, 0x00);
        // HiROM maps full 64 KiB banks linearly.
        assert_eq!(cart.map(0x40_0000), BsxBus::Psram(0));
        assert_eq!(cart.map(0x41_0000), BsxBus::Psram(0x1_0000));
        // $20-$3f / $a0-$bf:$6000-$7fff are 8 KiB PSRAM snippets.
        assert_eq!(cart.map(0x20_6000), BsxBus::Psram(0x6000));
        assert_eq!(cart.map(0x21_6000), BsxBus::Psram(0x1_6000));
        assert_eq!(cart.map(0xA0_6000), BsxBus::Psram(0x6000));
    }

    #[test]
    fn sram_and_mmio_are_always_mapped() {
        let cart = cartridge();
        assert_eq!(cart.map(0x10_5000), BsxBus::Sram(0));
        assert_eq!(cart.map(0x17_5FFF), BsxBus::Sram(0x7FFF));
        assert_eq!(cart.map(0x00_5000), BsxBus::Mmio(0));
        assert_eq!(cart.map(0x0F_5FFF), BsxBus::Mmio(0x0F));
        // $18-$3f:$5000-$5fff is not mapped.
        assert_eq!(cart.map(0x18_5000), BsxBus::Open);
    }

    #[test]
    fn sram_round_trips() {
        let mut cart = cartridge();
        assert!(cart.write(0x12_5123, 0xA5));
        assert_eq!(cart.read(0x12_5123, 0), 0xA5);
        assert_eq!(cart.read(0x12_5123, 0), cart.sram()[0x2123]);
    }

    #[test]
    fn pack_reads_and_write_protection() {
        let mut cart = cartridge();
        let pak = vec![0x5Au8; 0x10_0000];
        cart.load_pack(pak);
        // Default r02=1 (HiROM): $c0-$ff:$0000-$ffff is the pack.
        assert_eq!(cart.map(0xC0_0000), BsxBus::Pak(0));
        assert_eq!(cart.read(0xC0_0000, 0), 0x5A);
        // r0c=0: writes ignored.
        cart.write(0xC0_0000, 0x11);
        assert_eq!(cart.read(0xC0_0000, 0), 0x5A);
        // Enable write (r0c) and deassert /WP (r0d).
        commit(&mut cart, 0x0C, 0x80);
        commit(&mut cart, 0x0D, 0x80);
        cart.write(0xC0_0000, 0x11);
        assert_eq!(cart.read(0xC0_0000, 0), 0x11);
    }

    #[test]
    fn open_bus_where_no_device_is_mapped() {
        let cart = cartridge();
        assert_eq!(cart.read(0x7E_0000, 0x42), 0x42);
        assert_eq!(cart.read(0x00_0000, 0x42), 0x42);
    }
}
