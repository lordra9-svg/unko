//! Sufami Turbo base cartridge and its two slots.
//!
//! The Sufami Turbo is a Bandai add-on: a base cartridge (`ST-LOROM`, identified
//! by the embedded `BANDAI SFC-ADX` signature) exposes two small cartridge
//! slots. The base ROM occupies `$00-$1F/$80-$9F:$8000-$FFFF`; slot A's ROM is
//! at `$20-$3F/$A0-$BF:$8000-$FFFF` and its RAM at `$60-$6F/$E0-$EF:$0000-$FFFF`;
//! slot B's ROM is at `$40-$5F/$C0-$DF:$0000-$FFFF` and its RAM at
//! `$70-$7D/$F0-$FF:$0000-$FFFF`. Both slot ROMs and RAMs are mirrored with a
//! power-of-two wrap, exactly as ares' `SufamiTurboCartridge::read/write`.
//!
//! Like MSU1 media, the slot contents are host-attached: install them with
//! [`crate::Snes::set_sufami_turbo_slot`]. The base cartridge itself is an
//! ordinary LoROM image and is parsed as usual.

/// Signature at offset 0 of a Sufami Turbo base cartridge.
pub const SUFAMI_TURBO_SIGNATURE: &[u8] = b"BANDAI SFC-ADX";

/// One Sufami Turbo slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SufamiTurboSlot {
    /// Slot program ROM (host-attached, like the base cartridge ROM).
    #[serde(skip)]
    pub rom: Vec<u8>,
    /// Slot battery-backed RAM.
    pub ram: Vec<u8>,
}

impl SufamiTurboSlot {
    /// Read from the slot ROM, wrapping at its size (ares `mirror`).
    pub fn read_rom(&self, offset: u32) -> Option<u8> {
        if self.rom.is_empty() {
            return None;
        }
        let index = offset as usize % self.rom.len();
        self.rom.get(index).copied()
    }

    /// Read from the slot RAM, wrapping at its size.
    pub fn read_ram(&self, offset: u32) -> Option<u8> {
        if self.ram.is_empty() {
            return None;
        }
        let index = offset as usize % self.ram.len();
        self.ram.get(index).copied()
    }

    /// Write to the slot RAM, wrapping at its size.
    pub fn write_ram(&mut self, offset: u32, value: u8) {
        if self.ram.is_empty() {
            return;
        }
        let index = offset as usize % self.ram.len();
        if let Some(byte) = self.ram.get_mut(index) {
            *byte = value;
        }
    }
}

/// The two Sufami Turbo slots.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SufamiTurbo {
    /// Slot A and slot B, either may be empty.
    pub slots: [Option<SufamiTurboSlot>; 2],
}

impl SufamiTurbo {
    /// Install (or replace) one slot's media. `slot` is 0 (A) or 1 (B).
    pub fn set_slot(&mut self, slot: usize, rom: Vec<u8>, ram: Vec<u8>) {
        if slot > 1 {
            return;
        }
        self.slots[slot] = Some(SufamiTurboSlot { rom, ram });
    }

    /// Whether the address is a slot window, returning `(slot, is_rom, offset)`.
    ///
    /// The windows mirror the ares `ST-LOROM` board maps. `offset` is the
    /// address inside the window before the slot's power-of-two wrap.
    pub fn window(address: u32) -> Option<(usize, bool, u32)> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = address & 0xFFFF;
        let bank7 = bank & 0x7F;
        // Slot A ROM: $20-$3F/$A0-$BF:$8000-$FFFF. The 32 KiB window is
        // folded (`mask=0x8000`), so each bank advances the slot offset by
        // 32 KiB rather than 64 KiB.
        if (0x20..=0x3F).contains(&bank7) && offset >= 0x8000 {
            return Some((0, true, (u32::from(bank7 - 0x20) << 15) | (offset & 0x7FFF)));
        }
        // Slot B ROM: $40-$5F/$C0-$DF:$0000-$FFFF.
        if (0x40..=0x5F).contains(&bank7) {
            return Some((1, true, (u32::from(bank7 - 0x40) << 16) | offset));
        }
        // Slot A RAM: $60-$6F/$E0-$EF:$0000-$FFFF.
        if (0x60..=0x6F).contains(&bank7) {
            return Some((0, false, (u32::from(bank7 - 0x60) << 16) | offset));
        }
        // Slot B RAM: $70-$7D/$F0-$FF:$0000-$FFFF.
        if (0x70..=0x7D).contains(&bank7) {
            return Some((1, false, (u32::from(bank7 - 0x70) << 16) | offset));
        }
        None
    }

    /// Restore the battery-backed slot RAM while keeping host-attached ROMs.
    pub fn restore_state(&mut self, saved: &Self) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(saved_slot) = saved.slots[index].as_ref() else {
                *slot = None;
                continue;
            };
            match slot {
                Some(current) => current.ram.clone_from(&saved_slot.ram),
                None => {
                    *slot = Some(SufamiTurboSlot {
                        rom: Vec::new(),
                        ram: saved_slot.ram.clone(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_match_the_st_lorom_board() {
        assert_eq!(SufamiTurbo::window(0x20_8000), Some((0, true, 0)));
        assert_eq!(SufamiTurbo::window(0xA0_FFFF), Some((0, true, 0x7FFF)));
        assert_eq!(SufamiTurbo::window(0x41_1234), Some((1, true, 0x1_1234)));
        assert_eq!(SufamiTurbo::window(0x60_0000), Some((0, false, 0)));
        assert_eq!(SufamiTurbo::window(0xEF_FFFF), Some((0, false, 0xF_FFFF)));
        assert_eq!(SufamiTurbo::window(0x73_ABCD), Some((1, false, 0x3_ABCD)));
        assert_eq!(SufamiTurbo::window(0x1F_FFFF), None);
        assert_eq!(SufamiTurbo::window(0x20_7FFF), None);
        assert_eq!(SufamiTurbo::window(0x7E_0000), None);
    }

    #[test]
    fn slot_rom_and_ram_wrap_at_their_size() {
        let mut slot = SufamiTurboSlot {
            rom: vec![1, 2, 3, 4],
            ram: vec![0xAA, 0xBB],
        };
        assert_eq!(slot.read_rom(5), Some(2));
        assert_eq!(slot.read_ram(2), Some(0xAA));
        slot.write_ram(3, 0x55);
        assert_eq!(slot.read_ram(1), Some(0x55));
        assert_eq!(slot.read_rom(100), Some(1));
        assert_eq!(slot.read_rom(0x8000), Some(1));
    }

    #[test]
    fn empty_slots_read_as_open_bus() {
        let slot = SufamiTurboSlot::default();
        assert_eq!(slot.read_rom(0), None);
        assert_eq!(slot.read_ram(0), None);
    }

    #[test]
    fn restore_keeps_attached_roms_and_replaces_ram() {
        let mut current = SufamiTurbo::default();
        current.set_slot(0, vec![0x11; 4], vec![0; 4]);
        let mut saved = SufamiTurbo::default();
        saved.slots[0] = Some(SufamiTurboSlot {
            rom: Vec::new(),
            ram: vec![0x99; 4],
        });
        current.restore_state(&saved);
        let slot = current.slots[0].as_ref().unwrap();
        assert_eq!(slot.rom, vec![0x11; 4]);
        assert_eq!(slot.ram, vec![0x99; 4]);
    }
}
