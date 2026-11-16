//! S-CPU I/O: joypads, the hardware multiply/divide unit and auto-poll.
//!
//! The S-CPU exposes two controller ports through `$4016`/`$4017` (serial) and
//! `$4218-$421F` (auto-polled). This module keeps that state plus the
//! hardware multiply/divide unit at `$4202-$4206`/`$4214-$4217`, whose counters
//! advance once per S-CPU bus cycle. Behaviour follows the public SNES
//! documentation and the ISC-licensed ares `sfc/cpu` implementation.

/// Controller button layout used by [`CpuIo::joypad`].
///
/// The bit positions match the auto-poll result registers `$4218-$421F`, which
/// is also the order the serial ports shift out.
pub mod button {
    /// B.
    pub const B: u16 = 1 << 15;
    /// Y.
    pub const Y: u16 = 1 << 14;
    /// Select.
    pub const SELECT: u16 = 1 << 13;
    /// Start.
    pub const START: u16 = 1 << 12;
    /// Up.
    pub const UP: u16 = 1 << 11;
    /// Down.
    pub const DOWN: u16 = 1 << 10;
    /// Left.
    pub const LEFT: u16 = 1 << 9;
    /// Right.
    pub const RIGHT: u16 = 1 << 8;
    /// A.
    pub const A: u16 = 1 << 7;
    /// X.
    pub const X: u16 = 1 << 6;
    /// L.
    pub const L: u16 = 1 << 5;
    /// R.
    pub const R: u16 = 1 << 4;
}

/// The hardware multiply/divide unit (`$4202-$4206`, `$4214-$4217`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MathUnit {
    multiplicand: u8,
    multiplier: u8,
    dividend: u16,
    divisor: u8,
    multiply_counter: u32,
    divide_counter: u32,
    shift: u32,
    /// `$4214/$4215`: quotient after a divide, divisor latch after a multiply.
    quotient: u16,
    /// `$4216/$4217`: product after a multiply, remainder after a divide.
    product: u16,
}

impl MathUnit {
    /// Advance one S-CPU bus cycle.
    fn step(&mut self) {
        if self.multiply_counter != 0 {
            self.multiply_counter -= 1;
            if self.quotient & 1 != 0 {
                self.product = self.product.wrapping_add(self.shift as u16);
            }
            self.quotient >>= 1;
            self.shift <<= 1;
        }
        if self.divide_counter != 0 {
            self.divide_counter -= 1;
            self.quotient <<= 1;
            self.shift >>= 1;
            if u32::from(self.product) >= self.shift {
                self.product -= self.shift as u16;
                self.quotient |= 1;
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4202 => self.multiplicand = value,
            0x4203 => {
                self.product = 0;
                if self.multiply_counter != 0 || self.divide_counter != 0 {
                    return;
                }
                self.multiplier = value;
                self.quotient = (u16::from(self.multiplier) << 8) | u16::from(self.multiplicand);
                self.multiply_counter = 8;
                self.shift = u32::from(self.multiplier);
            }
            0x4204 => self.dividend = (self.dividend & 0xFF00) | u16::from(value),
            0x4205 => self.dividend = (self.dividend & 0x00FF) | (u16::from(value) << 8),
            0x4206 => {
                self.product = self.dividend;
                if self.multiply_counter != 0 || self.divide_counter != 0 {
                    return;
                }
                self.divisor = value;
                self.divide_counter = 16;
                self.shift = u32::from(value) << 16;
            }
            _ => {}
        }
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x4214 => self.quotient as u8,
            0x4215 => (self.quotient >> 8) as u8,
            0x4216 => self.product as u8,
            0x4217 => (self.product >> 8) as u8,
            _ => 0,
        }
    }
}

/// State of the controller ports and CPU-side I/O registers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuIo {
    /// Port 1 buttons, using the [`button`] layout.
    joypad1: u16,
    /// Port 2 buttons, using the [`button`] layout.
    joypad2: u16,
    shift1: u16,
    shift2: u16,
    latched: bool,
    /// `$4200` bit 0.
    auto_poll: bool,
    auto_counter: u8,
    auto_pad1: u16,
    auto_pad2: u16,
    auto_serial1: u16,
    auto_serial2: u16,
    auto_joy1: u16,
    auto_joy2: u16,
    auto_joy3: u16,
    auto_joy4: u16,
    math: MathUnit,
    /// `$4207/$4208`: horizontal IRQ compare value in dots.
    htime: u16,
    /// `$4209/$420A`: vertical IRQ compare line.
    vtime: u16,
    /// `$4200` bits 4/5.
    h_irq: bool,
    v_irq: bool,
    /// Latched IRQ condition, cleared by reading `$4211`.
    irq_flag: bool,
    /// Previous state of the IRQ compare condition, for edge detection.
    irq_condition: bool,
}

impl CpuIo {
    /// Create idle controller ports with no buttons pressed.
    pub const fn new() -> Self {
        Self {
            joypad1: 0,
            joypad2: 0,
            shift1: 0,
            shift2: 0,
            latched: false,
            auto_poll: false,
            auto_counter: 33,
            auto_pad1: 0,
            auto_pad2: 0,
            auto_serial1: 0,
            auto_serial2: 0,
            auto_joy1: 0,
            auto_joy2: 0,
            auto_joy3: 0,
            auto_joy4: 0,
            math: MathUnit {
                multiplicand: 0,
                multiplier: 0,
                dividend: 0,
                divisor: 0,
                multiply_counter: 0,
                divide_counter: 0,
                shift: 0,
                quotient: 0,
                product: 0,
            },
            htime: 0,
            vtime: 0,
            h_irq: false,
            v_irq: false,
            irq_flag: false,
            irq_condition: false,
        }
    }

    /// Set the buttons held on a controller port (`0` or `1`).
    pub fn set_joypad(&mut self, port: usize, state: u16) {
        if port == 0 {
            self.joypad1 = state;
        } else {
            self.joypad2 = state;
        }
    }

    /// Buttons held on a controller port (`0` or `1`).
    pub const fn joypad(&self, port: usize) -> u16 {
        if port == 0 {
            self.joypad1
        } else {
            self.joypad2
        }
    }

    /// Whether auto-polling is running (`$4212` bit 0).
    pub const fn auto_poll_busy(&self) -> bool {
        self.auto_poll && self.auto_counter < 33
    }

    /// Latched auto-poll results, packed as `$4218`/`$4219`.
    pub const fn auto_poll_joy1(&self) -> u16 {
        self.auto_joy1
    }

    /// Second controller's auto-poll result.
    pub const fn auto_poll_joy2(&self) -> u16 {
        self.auto_joy2
    }

    /// Reset to the power-on state, preserving the attached controllers.
    pub fn reset(&mut self) {
        let port1 = self.joypad1;
        let port2 = self.joypad2;
        *self = Self::new();
        self.joypad1 = port1;
        self.joypad2 = port2;
    }

    /// Start one hardware auto-poll sequence (called at vblank).
    pub fn begin_auto_poll(&mut self) {
        if !self.auto_poll {
            return;
        }
        self.auto_counter = 0;
        self.auto_joy1 = 0;
        self.auto_joy2 = 0;
        self.auto_joy3 = 0;
        self.auto_joy4 = 0;
        self.auto_serial1 = self.joypad1;
        self.auto_serial2 = self.joypad2;
    }

    /// Advance the auto-poll and math counters by `clocks` master clocks.
    pub fn step(&mut self, clocks: u32) {
        for _ in 0..(clocks / 6) {
            self.math.step();
        }
        self.auto_counter_step(clocks);
    }

    fn auto_counter_step(&mut self, clocks: u32) {
        if self.auto_counter >= 33 {
            return;
        }
        // Hardware shifts one bit every 256 master clocks and samples it on the
        // preceding 128-clock edge; approximate that with one 128-clock tick
        // per bit pair.
        let ticks = clocks / 128;
        for _ in 0..ticks {
            self.auto_counter = self.auto_counter.saturating_add(1);
            if self.auto_counter >= 33 {
                return;
            }
            // Bits are shifted in from counter 2 onward, one per two ticks.
            if self.auto_counter >= 2 && self.auto_counter & 1 == 0 {
                self.auto_joy1 = (self.auto_joy1 << 1) | ((self.auto_serial1 >> 15) & 1);
                self.auto_joy2 = (self.auto_joy2 << 1) | ((self.auto_serial2 >> 15) & 1);
                self.auto_joy3 = (self.auto_joy3 << 1) | 1;
                self.auto_joy4 = (self.auto_joy4 << 1) | 1;
                self.auto_serial1 <<= 1;
                self.auto_serial2 <<= 1;
            }
        }
    }

    /// Poll the H/V IRQ comparator for the current PPU position.
    ///
    /// The comparator fires on the rising edge of the compare condition, then
    /// the flag stays latched until `$4211` is read.
    pub fn poll_irq(&mut self, line: u16, dot: u16) -> bool {
        let v_match = !self.v_irq || line == self.vtime;
        let h_match = !self.h_irq || dot == (self.htime + 1);
        let condition = (self.h_irq || self.v_irq) && v_match && h_match;
        if condition && !self.irq_condition {
            self.irq_flag = true;
        }
        self.irq_condition = condition;
        self.irq_flag
    }

    /// Read a CPU I/O register in `$4016-$4017` or `$4210-$421F`.
    pub fn read(&mut self, address: u16) -> Option<u8> {
        let value = match address {
            0x4016 => {
                let bits = u8::from(self.shift1 & 0x8000 != 0)
                    | (u8::from(self.shift2 & 0x8000 != 0) << 1);
                if !self.latched {
                    self.shift1 = (self.shift1 << 1) | 1;
                    self.shift2 = (self.shift2 << 1) | 1;
                }
                bits
            }
            0x4017 => u8::from(self.shift2 & 0x8000 != 0) | 0b0000_0110,
            0x4210 => 0x02,
            0x4211 => {
                let value = if self.irq_flag { 0x80 } else { 0 };
                self.irq_flag = false;
                value
            }
            0x4212 => (u8::from(self.auto_poll_busy())) | 0x40,
            0x4213 => 0,
            0x4214..=0x4217 => self.math.read(address),
            0x4218 => self.auto_joy1 as u8,
            0x4219 => (self.auto_joy1 >> 8) as u8,
            0x421A => self.auto_joy2 as u8,
            0x421B => (self.auto_joy2 >> 8) as u8,
            0x421C => self.auto_joy3 as u8,
            0x421D => (self.auto_joy3 >> 8) as u8,
            0x421E => self.auto_joy4 as u8,
            0x421F => (self.auto_joy4 >> 8) as u8,
            _ => return None,
        };
        Some(value)
    }

    /// Write a CPU I/O register in `$4016` or `$4200-$4206`.
    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4016 => {
                self.latched = value & 0x01 != 0;
                if self.latched {
                    self.shift1 = self.joypad1;
                    self.shift2 = self.joypad2;
                }
            }
            0x4200 => {
                self.auto_poll = value & 0x01 != 0;
                self.h_irq = value & 0x10 != 0;
                self.v_irq = value & 0x20 != 0;
                if !self.h_irq && !self.v_irq {
                    self.irq_flag = false;
                    self.irq_condition = false;
                }
            }
            0x4207 => self.htime = (self.htime & 0x100) | u16::from(value),
            0x4208 => self.htime = (self.htime & 0x0FF) | (u16::from(value & 0x01) << 8),
            0x4209 => self.vtime = (self.vtime & 0x100) | u16::from(value),
            0x420A => self.vtime = (self.vtime & 0x0FF) | (u16::from(value & 0x01) << 8),
            0x4202..=0x4206 => self.math.write(address, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_read_returns_the_button_order() {
        let mut io = CpuIo::new();
        io.set_joypad(0, button::B | button::START | button::R);
        io.write(0x4016, 1);
        io.write(0x4016, 0);
        let mut bits = 0u16;
        for _ in 0..16 {
            bits = (bits << 1) | u16::from(io.read(0x4016).unwrap() & 1);
        }
        assert_eq!(bits, button::B | button::START | button::R);
        // After sixteen bits the port returns ones.
        assert_eq!(io.read(0x4016).unwrap() & 1, 1);
    }

    #[test]
    fn auto_poll_packs_the_same_bits() {
        let mut io = CpuIo::new();
        io.set_joypad(0, button::A | button::LEFT);
        io.write(0x4200, 1);
        io.begin_auto_poll();
        for _ in 0..64 {
            io.step(128);
        }
        assert_eq!(io.auto_poll_joy1(), button::A | button::LEFT);
        assert!(!io.auto_poll_busy());
    }

    #[test]
    fn v_irq_fires_once_per_frame() {
        let mut io = CpuIo::new();
        io.write(0x4209, 100);
        io.write(0x4200, 0x20);
        assert!(!io.poll_irq(99, 0));
        assert!(io.poll_irq(100, 0));
        assert!(io.read(0x4211).unwrap() & 0x80 != 0);
        // Acknowledged; the same line no longer asserts until the next frame.
        assert!(!io.poll_irq(100, 1));
        assert!(!io.poll_irq(101, 0));
    }

    #[test]
    fn h_and_v_irq_match_one_dot() {
        let mut io = CpuIo::new();
        io.write(0x4207, 50);
        io.write(0x4209, 120);
        io.write(0x4200, 0x30);
        assert!(!io.poll_irq(120, 50));
        assert!(io.poll_irq(120, 51));
        assert!(io.read(0x4211).unwrap() & 0x80 != 0);
        assert!(!io.poll_irq(120, 52));
    }

    #[test]
    fn multiply_takes_eight_cycles() {
        let mut io = CpuIo::new();
        io.write(0x4202, 0x05);
        io.write(0x4203, 0x07);
        assert_eq!(io.read(0x4216).unwrap(), 0);
        for _ in 0..8 {
            io.step(6);
        }
        assert_eq!(io.read(0x4216).unwrap(), 0x23);
        assert_eq!(io.read(0x4217).unwrap(), 0);
    }

    #[test]
    fn divide_takes_sixteen_cycles() {
        let mut io = CpuIo::new();
        io.write(0x4204, 0x64);
        io.write(0x4205, 0x00);
        io.write(0x4206, 0x07);
        for _ in 0..16 {
            io.step(6);
        }
        // 100 / 7 = 14 remainder 2. `$4214` reads the quotient and `$4216`
        // the remainder, matching ares `rddiv`/`rdmpy`.
        assert_eq!(io.read(0x4214).unwrap(), 14);
        assert_eq!(io.read(0x4216).unwrap(), 2);
    }
}
