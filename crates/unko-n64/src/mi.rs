//! Minimal MIPS Interface (MI) register block.
//!
//! Implements `MI_MODE`, `MI_VERSION`, `MI_INTERRUPT` and `MI_MASK`. The
//! documented interrupt bits 5:0 are readable/maskable; bit 6 is modelled as an
//! RCP NMI source (retail hardware leaves it undefined) so the CPU's NMI path
//! has a register-level trigger.

/// MI register block base.
pub const BASE: u32 = 0x0430_0000;

/// Modelled RCP NMI line in `MI_INTERRUPT`.
pub const NMI_BIT: u32 = 1 << 6;

/// One MIPS Interface.
#[derive(Debug, Clone, Default)]
pub struct Mi {
    mode: u32,
    interrupt: u32,
    mask: u32,
    nmi: bool,
}

impl Mi {
    /// Creates an idle MI.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads an MI register (`offset` from `0x0430_0000`).
    #[must_use]
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset & 0xF {
            0x0 => self.mode,
            0x4 => 0x0202_0102, // RSP/RDP/RAC/IO versions
            0x8 => self.interrupt | if self.nmi { NMI_BIT } else { 0 },
            0xC => self.mask,
            _ => 0,
        }
    }

    /// Writes an MI register. Returns true when the write asserts the modelled
    /// RCP NMI line (the caller forwards this to the CPU's NMI input).
    pub fn write_register(&mut self, offset: u32, value: u32) -> bool {
        match offset & 0xF {
            0x0 => {
                // Set/clear pairs for Upper/EBus/Repeat; keep the raw state.
                self.mode = value & 0x2F7F;
                false
            }
            0x8 => {
                // Documented source bits are write-1-to-clear; bit 6 models the
                // RCP NMI line (set on 1, cleared on 0).
                self.interrupt &= !(value & 0x3F);
                if value & NMI_BIT != 0 {
                    self.nmi = true;
                    self.interrupt |= NMI_BIT;
                    true
                } else {
                    self.nmi = false;
                    self.interrupt &= !NMI_BIT;
                    false
                }
            }
            0xC => {
                for bit in 0..6 {
                    let set = 1 << (bit * 2 + 1);
                    let clear = 1 << (bit * 2);
                    if value & set != 0 && value & clear == 0 {
                        self.mask |= 1 << bit;
                    } else if value & clear != 0 && value & set == 0 {
                        self.mask &= !(1 << bit);
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Raises a documented MI interrupt source bit (0 = SP .. 5 = DP).
    pub fn raise_interrupt(&mut self, bit: u32) {
        if bit < 6 {
            self.interrupt |= 1 << bit;
        }
    }

    /// Clears a documented MI interrupt source bit.
    pub fn clear_interrupt(&mut self, bit: u32) {
        if bit < 6 {
            self.interrupt &= !(1 << bit);
        }
    }

    /// Whether the modelled RCP NMI line is asserted.
    #[must_use]
    pub fn nmi_pending(&self) -> bool {
        self.nmi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_mask_and_nmi_trigger() {
        let mut mi = Mi::new();
        assert_eq!(mi.read_register(0x4), 0x0202_0102);
        // Set SP mask (bit 1), then clear it (bit 0).
        mi.write_register(0xC, 1 << 1);
        assert_eq!(mi.read_register(0xC) & 1, 1);
        mi.write_register(0xC, 1);
        assert_eq!(mi.read_register(0xC) & 1, 0);
        // Modelled RCP NMI source.
        assert!(mi.write_register(0x8, NMI_BIT));
        assert!(mi.nmi_pending());
        assert_ne!(mi.read_register(0x8) & NMI_BIT, 0);
        mi.write_register(0x8, 0);
        assert!(!mi.nmi_pending());
        assert_eq!(mi.read_register(0x8) & NMI_BIT, 0);
        // Documented sources are independent of the NMI bit.
        mi.raise_interrupt(2);
        assert_ne!(mi.read_register(0x8) & (1 << 2), 0);
        mi.clear_interrupt(2);
        assert_eq!(mi.read_register(0x8) & (1 << 2), 0);
        // Documented source bits are write-1-to-clear.
        mi.raise_interrupt(4);
        mi.write_register(0x8, 1 << 4);
        assert_eq!(mi.read_register(0x8) & (1 << 4), 0);
    }
}
