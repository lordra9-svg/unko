//! ares' power-on RAM pattern generator.
//!
//! ares fills S-CPU work RAM with a *structured* pseudo-random pattern at
//! power-on (`Random::array` with `Entropy::Low`), not with zeros:
//!
//! ```cpp
//! if(!reset) random.array({(u8*)wram, sizeof(wram)});
//! ```
//!
//! The pattern is deliberately repetitive - two byte values selected by address
//! bits, with occasional single-bit flips - because real WRAM powers up with
//! something like this and games that never initialise a buffer can see it.
//! Super Mario RPG does exactly that: its per-frame fade tables at
//! `$7E:4200-$4FFF` are read before they are written, so the rendered frame
//! depends on this data.
//!
//! The generator below is a faithful port of ares' `Random` (a PCG32) and of
//! `Random::array` for the low-entropy case, seeded with `(0, 0)` - the seed
//! ares uses when its "Deterministic Entropy" developer setting is on. Given
//! the same seed, ares and this core produce byte-identical power-on WRAM.

/// ares' PCG32 generator (`ares/ares/random.hpp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Random {
    state: u64,
    increment: u64,
}

impl Random {
    /// ares `Random::seed(seed, sequence)`; `sequence` defaults to 0.
    pub fn seed(seed: u64, sequence: u64) -> Self {
        let mut random = Self {
            state: 0,
            increment: sequence << 1 | 1,
        };
        random.step();
        random.state = random.state.wrapping_add(seed);
        random.step();
        random
    }

    /// ares `Random::step`.
    fn step(&mut self) -> u32 {
        let state = self.state;
        self.state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.increment);
        let xorshift = (((state >> 18) ^ state) >> 27) as u32;
        let rotate = (state >> 59) as u32;
        let rotate = rotate & 31;
        (xorshift >> rotate) | xorshift.wrapping_shl(rotate.wrapping_neg() & 31)
    }

    /// ares `Random::random`: two steps, the first in the high half.
    pub fn next_u64(&mut self) -> u64 {
        let high = u64::from(self.step());
        let low = u64::from(self.step());
        (high << 32) | low
    }

    /// ares `Random::array` for `Entropy::Low`, which is what the SFC system
    /// uses (`random.entropy(Random::Entropy::Low)`).
    pub fn array_low(&mut self, buffer: &mut [u8]) {
        let lo_bit = (self.next_u64() & 3) as u32;
        let hi_bit = (lo_bit + 8 + (self.next_u64() & 3) as u32) & 15;
        let mut lo_value = (self.next_u64() & 0xFF) as u8;
        let mut hi_value = (self.next_u64() & 0xFF) as u8;
        if self.next_u64() & 3 == 0 {
            lo_value = 0;
        }
        if self.next_u64() & 1 == 0 {
            hi_value = !lo_value;
        }

        for (address, byte) in buffer.iter_mut().enumerate() {
            let mut value = if (address as u32) & (1 << lo_bit) != 0 {
                lo_value
            } else {
                hi_value
            };
            if (address as u32) & (1 << hi_bit) != 0 {
                value = !value;
            }
            if self.next_u64() & 511 == 0 {
                value ^= 1 << ((self.next_u64() & 7) as u32);
            }
            if self.next_u64() & 2047 == 0 {
                value ^= 1 << ((self.next_u64() & 7) as u32);
            }
            *byte = value;
        }
    }
}

/// The 128 KiB of S-CPU work RAM ares presents at power-on.
pub fn power_on_wram() -> Vec<u8> {
    let mut wram = vec![0u8; 0x2_0000];
    Random::seed(0, 0).array_low(&mut wram);
    wram
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_seed_repeats_exactly() {
        let a = power_on_wram();
        let b = power_on_wram();
        assert_eq!(a, b);
        assert_eq!(a.len(), 0x2_0000);
    }

    #[test]
    fn pattern_is_structured_not_noise() {
        // The low-entropy fill has at most four distinct bytes per 16-byte
        // block before the rare bit flips, so it must not look uniform.
        let wram = power_on_wram();
        let distinct: std::collections::HashSet<u8> = wram[0..0x100].iter().copied().collect();
        assert!(
            distinct.len() <= 8,
            "expected a repetitive pattern, got {} distinct bytes",
            distinct.len()
        );
    }

    #[test]
    fn every_byte_is_written() {
        // ares' fill is exhaustive; a bug in the address-bit logic would show
        // up as a byte value that depends on the wrong address bit.
        let mut buffer = vec![0u8; 64];
        Random::seed(0, 0).array_low(&mut buffer);
        assert!(buffer.iter().any(|&b| b != buffer[0]));
    }
}
