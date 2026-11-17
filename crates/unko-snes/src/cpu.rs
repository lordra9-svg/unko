//! Cycle-stepped WDC 65C816 CPU.
//!
//! The interpreter executes one instruction per [`Cpu::step`] call and performs
//! every bus access through [`CpuBus`], so the bus sees the documented number
//! and order of cycles. Time is counted in NTSC master clocks: six per base
//! S-CPU bus cycle, plus the wait states reported by [`CpuBus::access_cycles`].
//!
//! Behaviour follows the public WDC 65C816 data sheet and the ISC-licensed ares
//! `wdc65816` implementation. The MIT `w65c816` crate is used only as a
//! cross-check oracle in tests.

use std::fmt;

/// A 24-bit system bus driven by the CPU.
pub trait CpuBus {
    /// Read one byte. The CPU accounts the clocks, not the bus.
    fn read(&mut self, address: u32) -> u8;
    /// Write one byte. The CPU accounts the clocks, not the bus.
    fn write(&mut self, address: u32, value: u8);
    /// Master clocks required by a memory access. The S-CPU runs one base bus
    /// cycle every six master clocks; slow ROM, WRAM and internal I/O add wait
    /// states, matching the ares `CPU::wait` table.
    fn access_cycles(&mut self, _address: u32) -> u32 {
        6
    }

    /// Advance one idle CPU cycle.
    fn idle(&mut self) {}
}

/// Processor status flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    /// Carry.
    pub carry: bool,
    /// Zero.
    pub zero: bool,
    /// Interrupt disable.
    pub irq_disable: bool,
    /// Decimal mode.
    pub decimal: bool,
    /// Index registers are 8-bit when set.
    pub index_8: bool,
    /// Accumulator/memory is 8-bit when set.
    pub mem_8: bool,
    /// Overflow.
    pub overflow: bool,
    /// Negative.
    pub negative: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            carry: false,
            zero: false,
            irq_disable: true,
            decimal: false,
            index_8: true,
            mem_8: true,
            overflow: false,
            negative: false,
        }
    }
}

/// WDC 65C816 register file and interpreter state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpu {
    /// Accumulator (C in 16-bit mode).
    pub a: u16,
    /// X index register.
    pub x: u16,
    /// Y index register.
    pub y: u16,
    /// Stack pointer.
    pub s: u16,
    /// Direct page register.
    pub d: u16,
    /// Program bank register.
    pub pbr: u8,
    /// Data bank register.
    pub dbr: u8,
    /// Program counter within `pbr`.
    pub pc: u16,
    /// Processor status flags.
    pub flags: Flags,
    /// Emulation mode.
    pub emulation: bool,
    /// Waiting for an interrupt (`WAI`).
    pub wai: bool,
    /// Stopped (`STP`).
    pub stp: bool,
    /// IRQ input line, active low in hardware terms; `true` means asserted.
    pub irq_line: bool,
    /// Latched NMI edge.
    pub nmi_pending: bool,
    /// Total master clocks retired.
    pub cycles: u64,
    /// Number of NMI edges serviced (debug).
    pub nmi_services: u64,
    /// Number of IRQ edges serviced (debug).
    pub irq_services: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    /// Create a CPU in its power-on state.
    pub const fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            s: 0x01FF,
            d: 0,
            pbr: 0,
            dbr: 0,
            pc: 0,
            flags: Flags {
                carry: false,
                zero: false,
                irq_disable: true,
                decimal: false,
                index_8: true,
                mem_8: true,
                overflow: false,
                negative: false,
            },
            emulation: true,
            wai: false,
            stp: false,
            irq_line: false,
            nmi_pending: false,
            cycles: 0,
            nmi_services: 0,
            irq_services: 0,
        }
    }

    /// Current 24-bit program counter.
    pub const fn pc_address(&self) -> u32 {
        ((self.pbr as u32) << 16) | self.pc as u32
    }

    /// Processor status as a byte.
    pub const fn p(&self) -> u8 {
        (self.flags.carry as u8)
            | ((self.flags.zero as u8) << 1)
            | ((self.flags.irq_disable as u8) << 2)
            | ((self.flags.decimal as u8) << 3)
            | ((self.flags.index_8 as u8) << 4)
            | ((self.flags.mem_8 as u8) << 5)
            | ((self.flags.overflow as u8) << 6)
            | ((self.flags.negative as u8) << 7)
    }

    /// Load the processor status from a byte.
    pub fn set_p(&mut self, value: u8) {
        self.flags.carry = value & 0x01 != 0;
        self.flags.zero = value & 0x02 != 0;
        self.flags.irq_disable = value & 0x04 != 0;
        self.flags.decimal = value & 0x08 != 0;
        self.flags.index_8 = value & 0x10 != 0;
        self.flags.mem_8 = value & 0x20 != 0;
        self.flags.overflow = value & 0x40 != 0;
        self.flags.negative = value & 0x80 != 0;
        self.normalize_flags();
    }

    /// Reset the CPU and fetch the reset vector.
    pub fn reset(&mut self, bus: &mut impl CpuBus) -> u32 {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.s = 0x01FF;
        self.d = 0;
        self.pbr = 0;
        self.dbr = 0;
        self.pc = 0;
        self.flags = Flags::default();
        self.emulation = true;
        self.wai = false;
        self.stp = false;
        self.irq_line = false;
        self.nmi_pending = false;
        let start = self.cycles;
        for _ in 0..7 {
            self.idle(bus);
        }
        let low = self.read(bus, 0x00FFFC);
        let high = self.read(bus, 0x00FFFD);
        self.pc = u16::from_le_bytes([low, high]);
        (self.cycles - start) as u32
    }

    /// Execute one instruction or service one pending interrupt.
    ///
    /// Returns the number of NTSC master clocks retired.
    pub fn step(&mut self, bus: &mut impl CpuBus) -> u32 {
        let start = self.cycles;

        if self.stp {
            self.idle(bus);
            return (self.cycles - start) as u32;
        }

        if self.wai {
            // WAI wakes on either interrupt line even when IRQ is masked; the
            // masked IRQ is not serviced, but execution resumes.
            if !(self.nmi_pending || self.irq_line) {
                self.idle(bus);
                return (self.cycles - start) as u32;
            }
            self.wai = false;
            self.idle(bus);
            return (self.cycles - start) as u32;
        }

        if self.nmi_pending {
            self.nmi_pending = false;
            self.nmi_services += 1;
            let vector = if self.emulation { 0xFFFA } else { 0xFFEA };
            self.service_interrupt(bus, vector, false);
            return (self.cycles - start) as u32;
        }

        if self.irq_line && !self.flags.irq_disable {
            self.irq_services += 1;
            let vector = if self.emulation { 0xFFFE } else { 0xFFEE };
            self.service_interrupt(bus, vector, false);
            return (self.cycles - start) as u32;
        }

        let opcode = self.fetch(bus);
        self.execute(bus, opcode);
        (self.cycles - start) as u32
    }

    /// Assert or release the IRQ input.
    pub fn set_irq(&mut self, asserted: bool) {
        self.irq_line = asserted;
    }

    /// Latch an NMI edge.
    pub fn trigger_nmi(&mut self) {
        self.nmi_pending = true;
    }

    fn interrupt_pending(&self) -> bool {
        self.nmi_pending || (self.irq_line && !self.flags.irq_disable)
    }

    fn normalize_flags(&mut self) {
        if self.emulation {
            self.flags.index_8 = true;
            self.flags.mem_8 = true;
            self.s = 0x0100 | (self.s & 0x00FF);
        }
        if self.flags.index_8 {
            self.x &= 0x00FF;
            self.y &= 0x00FF;
        }
    }

    fn read(&mut self, bus: &mut impl CpuBus, address: u32) -> u8 {
        let address = address & 0x00FF_FFFF;
        self.cycles += u64::from(bus.access_cycles(address));
        bus.read(address)
    }

    fn write(&mut self, bus: &mut impl CpuBus, address: u32, value: u8) {
        let address = address & 0x00FF_FFFF;
        self.cycles += u64::from(bus.access_cycles(address));
        bus.write(address, value);
    }

    fn idle(&mut self, bus: &mut impl CpuBus) {
        self.cycles += 6;
        bus.idle();
    }

    fn idle_irq(&mut self, bus: &mut impl CpuBus) {
        if self.interrupt_pending() {
            let address = self.pc_address();
            self.read(bus, address);
        } else {
            self.idle(bus);
        }
    }

    fn idle2(&mut self, bus: &mut impl CpuBus) {
        if self.d & 0x00FF != 0 {
            self.idle(bus);
        }
    }

    fn idle4(&mut self, bus: &mut impl CpuBus, from: u16, to: u16) {
        if !self.flags.index_8 || (from >> 8) != (to >> 8) {
            self.idle(bus);
        }
    }

    fn idle6(&mut self, bus: &mut impl CpuBus, target: u16) {
        if self.emulation && (self.pc >> 8) != (target >> 8) {
            self.idle(bus);
        }
    }

    fn fetch(&mut self, bus: &mut impl CpuBus) -> u8 {
        let value = self.read(bus, self.pc_address());
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn fetch16(&mut self, bus: &mut impl CpuBus) -> u16 {
        let low = self.fetch(bus);
        let high = self.fetch(bus);
        u16::from_le_bytes([low, high])
    }

    fn fetch24(&mut self, bus: &mut impl CpuBus) -> u32 {
        let low = self.fetch(bus) as u32;
        let high = self.fetch(bus) as u32;
        let bank = self.fetch(bus) as u32;
        (bank << 16) | (high << 8) | low
    }

    fn read_direct(&mut self, bus: &mut impl CpuBus, offset: u16) -> u8 {
        if self.emulation && self.d & 0x00FF == 0 {
            let address = (self.d & 0xFF00) | (offset & 0x00FF);
            self.read(bus, address as u32)
        } else {
            self.read(bus, self.d.wrapping_add(offset) as u32)
        }
    }

    fn read_direct_n(&mut self, bus: &mut impl CpuBus, offset: u16) -> u8 {
        self.read(bus, self.d.wrapping_add(offset) as u32)
    }

    fn read_direct_x(
        &mut self,
        bus: &mut impl CpuBus,
        base: u16,
        index: u16,
        byte_offset: u16,
    ) -> u8 {
        if self.emulation && self.d & 0x00FF != 0 {
            let sum = self.d.wrapping_add(base);
            let low = sum.wrapping_add(index).wrapping_add(byte_offset) as u8;
            let address = (sum & 0xFF00) | u16::from(low);
            self.read(bus, address as u32)
        } else {
            self.read_direct(bus, base.wrapping_add(index).wrapping_add(byte_offset))
        }
    }

    fn read_direct16(&mut self, bus: &mut impl CpuBus, base: u16) -> u16 {
        let low = self.read_direct(bus, base);
        let high = self.read_direct(bus, base.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn read_bank(&mut self, bus: &mut impl CpuBus, offset: u16) -> u8 {
        self.read(bus, ((self.dbr as u32) << 16) | offset as u32)
    }

    fn read_bank16(&mut self, bus: &mut impl CpuBus, offset: u16) -> u16 {
        let low = self.read_bank(bus, offset);
        let high = self.read_bank(bus, offset.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn read_stack(&mut self, bus: &mut impl CpuBus, offset: u16) -> u8 {
        self.read(bus, self.s.wrapping_add(offset) as u32)
    }

    fn read_stack16(&mut self, bus: &mut impl CpuBus, offset: u16) -> u16 {
        let low = self.read_stack(bus, offset);
        let high = self.read_stack(bus, offset.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn write_direct(&mut self, bus: &mut impl CpuBus, offset: u16, value: u8) {
        if self.emulation && self.d & 0x00FF == 0 {
            let address = (self.d & 0xFF00) | (offset & 0x00FF);
            self.write(bus, address as u32, value);
        } else {
            self.write(bus, self.d.wrapping_add(offset) as u32, value);
        }
    }

    fn write_bank(&mut self, bus: &mut impl CpuBus, offset: u16, value: u8) {
        self.write(bus, ((self.dbr as u32) << 16) | offset as u32, value);
    }

    fn push(&mut self, bus: &mut impl CpuBus, value: u8) {
        let address = self.s;
        self.write(bus, address as u32, value);
        self.s = if self.emulation {
            0x0100 | (self.s.wrapping_sub(1) & 0x00FF)
        } else {
            self.s.wrapping_sub(1)
        };
    }

    fn pull(&mut self, bus: &mut impl CpuBus) -> u8 {
        self.s = if self.emulation {
            0x0100 | (self.s.wrapping_add(1) & 0x00FF)
        } else {
            self.s.wrapping_add(1)
        };
        self.read(bus, self.s as u32)
    }

    fn push_n(&mut self, bus: &mut impl CpuBus, value: u8) {
        let address = self.s;
        self.write(bus, address as u32, value);
        self.s = self.s.wrapping_sub(1);
    }

    fn pull_n(&mut self, bus: &mut impl CpuBus) -> u8 {
        self.s = self.s.wrapping_add(1);
        self.read(bus, self.s as u32)
    }

    fn service_interrupt(&mut self, bus: &mut impl CpuBus, vector: u16, software: bool) {
        if software {
            self.fetch(bus);
        } else {
            let address = self.pc_address();
            self.read(bus, address);
            self.idle(bus);
        }
        if !self.emulation {
            self.push(bus, self.pbr);
        }
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        let p = if software {
            self.p()
        } else if self.emulation {
            self.p() & !0x10
        } else {
            self.p()
        };
        self.push(bus, p);
        self.flags.irq_disable = true;
        self.flags.decimal = false;
        let low = self.read(bus, vector as u32);
        let high = self.read(bus, vector as u32 + 1);
        self.pc = u16::from_le_bytes([low, high]);
        self.pbr = 0;
    }

    fn set_a8(&mut self, value: u8) {
        self.a = (self.a & 0xFF00) | u16::from(value);
    }

    fn a8(&self) -> u8 {
        self.a as u8
    }

    fn set_zn8(&mut self, value: u8) {
        self.flags.zero = value == 0;
        self.flags.negative = value & 0x80 != 0;
    }

    fn set_zn16(&mut self, value: u16) {
        self.flags.zero = value == 0;
        self.flags.negative = value & 0x8000 != 0;
    }

    fn adc8(&mut self, data: u8) {
        let a = self.a8();
        let mut result: i32;
        if !self.flags.decimal {
            result = i32::from(a) + i32::from(data) + i32::from(self.flags.carry);
        } else {
            result = i32::from(a & 0x0F) + i32::from(data & 0x0F) + i32::from(self.flags.carry);
            if result > 0x09 {
                result += 0x06;
            }
            self.flags.carry = result > 0x0F;
            result = i32::from(a & 0xF0)
                + i32::from(data & 0xF0)
                + (i32::from(self.flags.carry) << 4)
                + (result & 0x0F);
        }
        self.flags.overflow = (!(a ^ data) & (a ^ (result as u8)) & 0x80) != 0;
        if self.flags.decimal && result > 0x9F {
            result += 0x60;
        }
        self.flags.carry = result > 0xFF;
        let result = result as u8;
        self.set_a8(result);
        self.set_zn8(result);
    }

    fn adc16(&mut self, data: u16) {
        let a = self.a;
        let mut result: i32;
        if !self.flags.decimal {
            result = i32::from(a) + i32::from(data) + i32::from(self.flags.carry);
        } else {
            result = i32::from(a & 0x000F) + i32::from(data & 0x000F) + i32::from(self.flags.carry);
            if result > 0x0009 {
                result += 0x0006;
            }
            self.flags.carry = result > 0x000F;
            result = i32::from(a & 0x00F0)
                + i32::from(data & 0x00F0)
                + (i32::from(self.flags.carry) << 4)
                + (result & 0x000F);
            if result > 0x009F {
                result += 0x0060;
            }
            self.flags.carry = result > 0x00FF;
            result = i32::from(a & 0x0F00)
                + i32::from(data & 0x0F00)
                + (i32::from(self.flags.carry) << 8)
                + (result & 0x00FF);
            if result > 0x09FF {
                result += 0x0600;
            }
            self.flags.carry = result > 0x0FFF;
            result = i32::from(a & 0xF000)
                + i32::from(data & 0xF000)
                + (i32::from(self.flags.carry) << 12)
                + (result & 0x0FFF);
        }
        self.flags.overflow = (!(a ^ data) & (a ^ (result as u16)) & 0x8000) != 0;
        if self.flags.decimal && result > 0x9FFF {
            result += 0x6000;
        }
        self.flags.carry = result > 0xFFFF;
        let result = result as u16;
        self.a = result;
        self.set_zn16(result);
    }

    fn sbc8(&mut self, data: u8) {
        let a = self.a8();
        let data = !data;
        let mut result: i32;
        if !self.flags.decimal {
            result = i32::from(a) + i32::from(data) + i32::from(self.flags.carry);
        } else {
            result = i32::from(a & 0x0F) + i32::from(data & 0x0F) + i32::from(self.flags.carry);
            if result <= 0x0F {
                result -= 0x06;
            }
            self.flags.carry = result > 0x0F;
            result = i32::from(a & 0xF0)
                + i32::from(data & 0xF0)
                + (i32::from(self.flags.carry) << 4)
                + (result & 0x0F);
        }
        self.flags.overflow = (!(a ^ data) & (a ^ (result as u8)) & 0x80) != 0;
        if self.flags.decimal && result <= 0xFF {
            result -= 0x60;
        }
        self.flags.carry = result > 0xFF;
        let result = result as u8;
        self.set_a8(result);
        self.set_zn8(result);
    }

    fn sbc16(&mut self, data: u16) {
        let a = self.a;
        let data = !data;
        let mut result: i32;
        if !self.flags.decimal {
            result = i32::from(a) + i32::from(data) + i32::from(self.flags.carry);
        } else {
            result = i32::from(a & 0x000F) + i32::from(data & 0x000F) + i32::from(self.flags.carry);
            if result <= 0x000F {
                result -= 0x0006;
            }
            self.flags.carry = result > 0x000F;
            result = i32::from(a & 0x00F0)
                + i32::from(data & 0x00F0)
                + (i32::from(self.flags.carry) << 4)
                + (result & 0x000F);
            if result <= 0x00FF {
                result -= 0x0060;
            }
            self.flags.carry = result > 0x00FF;
            result = i32::from(a & 0x0F00)
                + i32::from(data & 0x0F00)
                + (i32::from(self.flags.carry) << 8)
                + (result & 0x00FF);
            if result <= 0x0FFF {
                result -= 0x0600;
            }
            self.flags.carry = result > 0x0FFF;
            result = i32::from(a & 0xF000)
                + i32::from(data & 0xF000)
                + (i32::from(self.flags.carry) << 12)
                + (result & 0x0FFF);
        }
        self.flags.overflow = (!(a ^ data) & (a ^ (result as u16)) & 0x8000) != 0;
        if self.flags.decimal && result <= 0xFFFF {
            result -= 0x6000;
        }
        self.flags.carry = result > 0xFFFF;
        let result = result as u16;
        self.a = result;
        self.set_zn16(result);
    }

    fn cmp8(&mut self, left: u8, right: u8) {
        let result = i16::from(left) - i16::from(right);
        self.flags.carry = result >= 0;
        self.flags.zero = result as u8 == 0;
        self.flags.negative = result as u8 & 0x80 != 0;
    }

    fn cmp16(&mut self, left: u16, right: u16) {
        let result = i32::from(left) - i32::from(right);
        self.flags.carry = result >= 0;
        self.flags.zero = result as u16 == 0;
        self.flags.negative = result as u16 & 0x8000 != 0;
    }

    fn apply_read8(&mut self, op: ReadOp, value: u8) {
        match op {
            ReadOp::Ora => {
                let result = self.a8() | value;
                self.set_a8(result);
                self.set_zn8(result);
            }
            ReadOp::And => {
                let result = self.a8() & value;
                self.set_a8(result);
                self.set_zn8(result);
            }
            ReadOp::Eor => {
                let result = self.a8() ^ value;
                self.set_a8(result);
                self.set_zn8(result);
            }
            ReadOp::Adc => self.adc8(value),
            ReadOp::Lda => {
                self.set_a8(value);
                self.set_zn8(value);
            }
            ReadOp::Cmp => self.cmp8(self.a8(), value),
            ReadOp::Sbc => self.sbc8(value),
            ReadOp::Bit => {
                self.flags.zero = self.a8() & value == 0;
                self.flags.overflow = value & 0x40 != 0;
                self.flags.negative = value & 0x80 != 0;
            }
        }
    }

    fn apply_read16(&mut self, op: ReadOp, value: u16) {
        match op {
            ReadOp::Ora => {
                self.a |= value;
                self.set_zn16(self.a);
            }
            ReadOp::And => {
                self.a &= value;
                self.set_zn16(self.a);
            }
            ReadOp::Eor => {
                self.a ^= value;
                self.set_zn16(self.a);
            }
            ReadOp::Adc => self.adc16(value),
            ReadOp::Lda => {
                self.a = value;
                self.set_zn16(value);
            }
            ReadOp::Cmp => self.cmp16(self.a, value),
            ReadOp::Sbc => self.sbc16(value),
            ReadOp::Bit => {
                self.flags.zero = self.a & value == 0;
                self.flags.overflow = value & 0x4000 != 0;
                self.flags.negative = value & 0x8000 != 0;
            }
        }
    }

    fn apply_index8(&mut self, op: IndexOp, value: u8) {
        match op {
            IndexOp::Ldx => {
                self.x = (self.x & 0xFF00) | u16::from(value);
                self.set_zn8(value);
            }
            IndexOp::Ldy => {
                self.y = (self.y & 0xFF00) | u16::from(value);
                self.set_zn8(value);
            }
            IndexOp::Cpx => self.cmp8(self.x as u8, value),
            IndexOp::Cpy => self.cmp8(self.y as u8, value),
        }
    }

    fn apply_index16(&mut self, op: IndexOp, value: u16) {
        match op {
            IndexOp::Ldx => {
                self.x = value;
                self.set_zn16(value);
            }
            IndexOp::Ldy => {
                self.y = value;
                self.set_zn16(value);
            }
            IndexOp::Cpx => self.cmp16(self.x, value),
            IndexOp::Cpy => self.cmp16(self.y, value),
        }
    }

    fn apply_modify8(&mut self, op: ModifyOp, value: u8) -> u8 {
        match op {
            ModifyOp::Asl => {
                self.flags.carry = value & 0x80 != 0;
                let result = value << 1;
                self.set_zn8(result);
                result
            }
            ModifyOp::Lsr => {
                self.flags.carry = value & 0x01 != 0;
                let result = value >> 1;
                self.set_zn8(result);
                result
            }
            ModifyOp::Rol => {
                let carry = self.flags.carry;
                self.flags.carry = value & 0x80 != 0;
                let result = (value << 1) | u8::from(carry);
                self.set_zn8(result);
                result
            }
            ModifyOp::Ror => {
                let carry = self.flags.carry;
                self.flags.carry = value & 0x01 != 0;
                let result = (u8::from(carry) << 7) | (value >> 1);
                self.set_zn8(result);
                result
            }
            ModifyOp::Inc => {
                let result = value.wrapping_add(1);
                self.set_zn8(result);
                result
            }
            ModifyOp::Dec => {
                let result = value.wrapping_sub(1);
                self.set_zn8(result);
                result
            }
            ModifyOp::Tsb => {
                self.flags.zero = value & self.a8() == 0;
                value | self.a8()
            }
            ModifyOp::Trb => {
                self.flags.zero = value & self.a8() == 0;
                value & !self.a8()
            }
        }
    }

    fn apply_modify16(&mut self, op: ModifyOp, value: u16) -> u16 {
        match op {
            ModifyOp::Asl => {
                self.flags.carry = value & 0x8000 != 0;
                let result = value << 1;
                self.set_zn16(result);
                result
            }
            ModifyOp::Lsr => {
                self.flags.carry = value & 0x0001 != 0;
                let result = value >> 1;
                self.set_zn16(result);
                result
            }
            ModifyOp::Rol => {
                let carry = self.flags.carry;
                self.flags.carry = value & 0x8000 != 0;
                let result = (value << 1) | u16::from(carry);
                self.set_zn16(result);
                result
            }
            ModifyOp::Ror => {
                let carry = self.flags.carry;
                self.flags.carry = value & 0x0001 != 0;
                let result = (u16::from(carry) << 15) | (value >> 1);
                self.set_zn16(result);
                result
            }
            ModifyOp::Inc => {
                let result = value.wrapping_add(1);
                self.set_zn16(result);
                result
            }
            ModifyOp::Dec => {
                let result = value.wrapping_sub(1);
                self.set_zn16(result);
                result
            }
            ModifyOp::Tsb => {
                self.flags.zero = value & self.a == 0;
                value | self.a
            }
            ModifyOp::Trb => {
                self.flags.zero = value & self.a == 0;
                value & !self.a
            }
        }
    }

    fn read_op(&mut self, bus: &mut impl CpuBus, op: ReadOp, mode: Mode) {
        let eight = self.flags.mem_8;
        let value = match mode {
            Mode::Immediate => {
                let low = self.fetch(bus);
                if eight {
                    u16::from(low)
                } else {
                    let high = self.fetch(bus);
                    u16::from_le_bytes([low, high])
                }
            }
            Mode::Direct => {
                let base = self.fetch(bus);
                self.idle2(bus);
                if eight {
                    u16::from(self.read_direct(bus, u16::from(base)))
                } else {
                    self.read_direct16(bus, u16::from(base))
                }
            }
            Mode::DirectX | Mode::DirectY => {
                let base = self.fetch(bus);
                let index = if mode == Mode::DirectX {
                    self.x
                } else {
                    self.y
                };
                self.idle2(bus);
                self.idle(bus);
                if eight {
                    u16::from(self.read_direct(bus, u16::from(base).wrapping_add(index)))
                } else {
                    let offset = u16::from(base).wrapping_add(index);
                    self.read_direct16(bus, offset)
                }
            }
            Mode::Bank => {
                let base = self.fetch16(bus);
                if eight {
                    u16::from(self.read_bank(bus, base))
                } else {
                    self.read_bank16(bus, base)
                }
            }
            Mode::BankX | Mode::BankY => {
                let base = self.fetch16(bus);
                let index = if mode == Mode::BankX { self.x } else { self.y };
                self.idle4(bus, base, base.wrapping_add(index));
                let address = base.wrapping_add(index);
                if eight {
                    u16::from(self.read_bank(bus, address))
                } else {
                    self.read_bank16(bus, address)
                }
            }
            Mode::Long => {
                let address = self.fetch24(bus);
                if eight {
                    u16::from(self.read(bus, address))
                } else {
                    let low = self.read(bus, address);
                    let high = self.read(bus, address.wrapping_add(1));
                    u16::from_le_bytes([low, high])
                }
            }
            Mode::LongX => {
                let address = self.fetch24(bus).wrapping_add(self.x as u32);
                if eight {
                    u16::from(self.read(bus, address))
                } else {
                    let low = self.read(bus, address);
                    let high = self.read(bus, address.wrapping_add(1));
                    u16::from_le_bytes([low, high])
                }
            }
            Mode::Indirect => {
                let base = self.fetch(bus);
                self.idle2(bus);
                let pointer = self.read_direct16(bus, u16::from(base));
                if eight {
                    u16::from(self.read_bank(bus, pointer))
                } else {
                    self.read_bank16(bus, pointer)
                }
            }
            Mode::IndexedIndirect => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                self.idle(bus);
                let low = self.read_direct_x(bus, base, self.x, 0);
                let high = self.read_direct_x(bus, base, self.x, 1);
                let pointer = u16::from_le_bytes([low, high]);
                if eight {
                    u16::from(self.read_bank(bus, pointer))
                } else {
                    self.read_bank16(bus, pointer)
                }
            }
            Mode::IndirectIndexed => {
                let base = self.fetch(bus);
                self.idle2(bus);
                let pointer = self.read_direct16(bus, u16::from(base));
                self.idle4(bus, pointer, pointer.wrapping_add(self.y));
                let address = pointer.wrapping_add(self.y);
                if eight {
                    u16::from(self.read_bank(bus, address))
                } else {
                    self.read_bank16(bus, address)
                }
            }
            Mode::IndirectLong | Mode::IndirectLongY => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                let low = self.read_direct_n(bus, base);
                let high = self.read_direct_n(bus, base.wrapping_add(1));
                let bank = self.read_direct_n(bus, base.wrapping_add(2));
                let mut address = (u32::from(bank) << 16) | (u32::from(high) << 8) | u32::from(low);
                if mode == Mode::IndirectLongY {
                    address = address.wrapping_add(self.y as u32);
                }
                if eight {
                    u16::from(self.read(bus, address))
                } else {
                    let low = self.read(bus, address);
                    let high = self.read(bus, address.wrapping_add(1));
                    u16::from_le_bytes([low, high])
                }
            }
            Mode::Stack => {
                let base = u16::from(self.fetch(bus));
                self.idle(bus);
                if eight {
                    u16::from(self.read_stack(bus, base))
                } else {
                    self.read_stack16(bus, base)
                }
            }
            Mode::IndirectStackY => {
                let base = u16::from(self.fetch(bus));
                self.idle(bus);
                let pointer = self.read_stack16(bus, base);
                self.idle(bus);
                let address = pointer.wrapping_add(self.y);
                if eight {
                    u16::from(self.read_bank(bus, address))
                } else {
                    self.read_bank16(bus, address)
                }
            }
            Mode::Implied => unreachable!("implied read"),
        };
        if eight {
            self.apply_read8(op, value as u8);
        } else {
            self.apply_read16(op, value);
        }
    }

    fn read_index_op(&mut self, bus: &mut impl CpuBus, op: IndexOp, mode: Mode) {
        let eight = self.flags.index_8;
        let value = match mode {
            Mode::Immediate => {
                let low = self.fetch(bus);
                if eight {
                    u16::from(low)
                } else {
                    let high = self.fetch(bus);
                    u16::from_le_bytes([low, high])
                }
            }
            Mode::Direct => {
                let base = self.fetch(bus);
                self.idle2(bus);
                if eight {
                    u16::from(self.read_direct(bus, u16::from(base)))
                } else {
                    self.read_direct16(bus, u16::from(base))
                }
            }
            Mode::DirectX | Mode::DirectY => {
                let base = self.fetch(bus);
                let index = if mode == Mode::DirectX {
                    self.x
                } else {
                    self.y
                };
                self.idle2(bus);
                self.idle(bus);
                let offset = u16::from(base).wrapping_add(index);
                if eight {
                    u16::from(self.read_direct(bus, offset))
                } else {
                    self.read_direct16(bus, offset)
                }
            }
            Mode::Bank => {
                let base = self.fetch16(bus);
                if eight {
                    u16::from(self.read_bank(bus, base))
                } else {
                    self.read_bank16(bus, base)
                }
            }
            Mode::BankX | Mode::BankY => {
                let base = self.fetch16(bus);
                let index = if mode == Mode::BankX { self.x } else { self.y };
                self.idle4(bus, base, base.wrapping_add(index));
                let address = base.wrapping_add(index);
                if eight {
                    u16::from(self.read_bank(bus, address))
                } else {
                    self.read_bank16(bus, address)
                }
            }
            _ => unreachable!("invalid index addressing mode"),
        };
        if eight {
            self.apply_index8(op, value as u8);
        } else {
            self.apply_index16(op, value);
        }
    }

    fn write_op(&mut self, bus: &mut impl CpuBus, op: WriteOp, mode: Mode) {
        let width_is_index = matches!(op, WriteOp::Stx | WriteOp::Sty);
        let eight = if width_is_index {
            self.flags.index_8
        } else {
            self.flags.mem_8
        };
        let value = match op {
            WriteOp::Sta => self.a,
            WriteOp::Stz => 0,
            WriteOp::Stx => self.x,
            WriteOp::Sty => self.y,
        };
        match mode {
            Mode::Direct => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                self.write_direct(bus, base, value as u8);
                if !eight {
                    self.write_direct(bus, base.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::DirectX | Mode::DirectY => {
                let base = u16::from(self.fetch(bus));
                let index = if mode == Mode::DirectX {
                    self.x
                } else {
                    self.y
                };
                self.idle2(bus);
                self.idle(bus);
                let address = base.wrapping_add(index);
                self.write_direct(bus, address, value as u8);
                if !eight {
                    self.write_direct(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::Bank => {
                let base = self.fetch16(bus);
                self.write_bank(bus, base, value as u8);
                if !eight {
                    self.write_bank(bus, base.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::BankX | Mode::BankY => {
                let base = self.fetch16(bus);
                let index = if mode == Mode::BankX { self.x } else { self.y };
                self.idle(bus);
                let address = base.wrapping_add(index);
                self.write_bank(bus, address, value as u8);
                if !eight {
                    self.write_bank(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::Long | Mode::LongX => {
                let mut address = self.fetch24(bus);
                if mode == Mode::LongX {
                    address = address.wrapping_add(self.x as u32);
                }
                self.write(bus, address, value as u8);
                if !eight {
                    self.write(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::Indirect => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                let pointer = self.read_direct16(bus, base);
                self.write_bank(bus, pointer, value as u8);
                if !eight {
                    self.write_bank(bus, pointer.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::IndexedIndirect => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                self.idle(bus);
                let low = self.read_direct_x(bus, base, self.x, 0);
                let high = self.read_direct_x(bus, base, self.x, 1);
                let pointer = u16::from_le_bytes([low, high]);
                self.write_bank(bus, pointer, value as u8);
                if !eight {
                    self.write_bank(bus, pointer.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::IndirectIndexed => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                let pointer = self.read_direct16(bus, base);
                self.idle(bus);
                let address = pointer.wrapping_add(self.y);
                self.write_bank(bus, address, value as u8);
                if !eight {
                    self.write_bank(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::IndirectLong | Mode::IndirectLongY => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                let low = self.read_direct_n(bus, base);
                let high = self.read_direct_n(bus, base.wrapping_add(1));
                let bank = self.read_direct_n(bus, base.wrapping_add(2));
                let mut address = (u32::from(bank) << 16) | (u32::from(high) << 8) | u32::from(low);
                if mode == Mode::IndirectLongY {
                    address = address.wrapping_add(self.y as u32);
                }
                self.write(bus, address, value as u8);
                if !eight {
                    self.write(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::Stack => {
                let base = u16::from(self.fetch(bus));
                self.idle(bus);
                self.write_stack(bus, base, value as u8);
                if !eight {
                    self.write_stack(bus, base.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::IndirectStackY => {
                let base = u16::from(self.fetch(bus));
                self.idle(bus);
                let pointer = self.read_stack16(bus, base);
                self.idle(bus);
                let address = pointer.wrapping_add(self.y);
                self.write_bank(bus, address, value as u8);
                if !eight {
                    self.write_bank(bus, address.wrapping_add(1), (value >> 8) as u8);
                }
            }
            Mode::Immediate => unreachable!("immediate write"),
            Mode::Implied => unreachable!("implied write"),
        }
    }

    fn write_stack(&mut self, bus: &mut impl CpuBus, offset: u16, value: u8) {
        self.write(bus, self.s.wrapping_add(offset) as u32, value);
    }

    fn modify_op(&mut self, bus: &mut impl CpuBus, op: ModifyOp, mode: Mode) {
        let eight = self.flags.mem_8;
        match mode {
            Mode::Implied => {
                self.idle_irq(bus);
                if eight {
                    let result = self.apply_modify8(op, self.a8());
                    self.set_a8(result);
                } else {
                    let result = self.apply_modify16(op, self.a);
                    self.a = result;
                }
            }
            Mode::Direct => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                if eight {
                    let value = self.read_direct(bus, base);
                    self.idle(bus);
                    let result = self.apply_modify8(op, value);
                    self.write_direct(bus, base, result);
                } else {
                    let value = self.read_direct16(bus, base);
                    self.idle(bus);
                    let result = self.apply_modify16(op, value);
                    self.write_direct(bus, base.wrapping_add(1), (result >> 8) as u8);
                    self.write_direct(bus, base, result as u8);
                }
            }
            Mode::DirectX => {
                let base = u16::from(self.fetch(bus));
                self.idle2(bus);
                self.idle(bus);
                let address = base.wrapping_add(self.x);
                if eight {
                    let value = self.read_direct(bus, address);
                    self.idle(bus);
                    let result = self.apply_modify8(op, value);
                    self.write_direct(bus, address, result);
                } else {
                    let value = self.read_direct16(bus, address);
                    self.idle(bus);
                    let result = self.apply_modify16(op, value);
                    self.write_direct(bus, address.wrapping_add(1), (result >> 8) as u8);
                    self.write_direct(bus, address, result as u8);
                }
            }
            Mode::Bank => {
                let base = self.fetch16(bus);
                if eight {
                    let value = self.read_bank(bus, base);
                    self.idle(bus);
                    let result = self.apply_modify8(op, value);
                    self.write_bank(bus, base, result);
                } else {
                    let value = self.read_bank16(bus, base);
                    self.idle(bus);
                    let result = self.apply_modify16(op, value);
                    self.write_bank(bus, base.wrapping_add(1), (result >> 8) as u8);
                    self.write_bank(bus, base, result as u8);
                }
            }
            Mode::BankX => {
                let base = self.fetch16(bus);
                self.idle(bus);
                let address = base.wrapping_add(self.x);
                if eight {
                    let value = self.read_bank(bus, address);
                    self.idle(bus);
                    let result = self.apply_modify8(op, value);
                    self.write_bank(bus, address, result);
                } else {
                    let value = self.read_bank16(bus, address);
                    self.idle(bus);
                    let result = self.apply_modify16(op, value);
                    self.write_bank(bus, address.wrapping_add(1), (result >> 8) as u8);
                    self.write_bank(bus, address, result as u8);
                }
            }
            _ => unreachable!("invalid modify addressing mode"),
        }
    }

    fn bit_immediate(&mut self, bus: &mut impl CpuBus) {
        if self.flags.mem_8 {
            let value = self.fetch(bus);
            self.flags.zero = self.a8() & value == 0;
        } else {
            let value = self.fetch16(bus);
            self.flags.zero = self.a & value == 0;
        }
    }

    fn branch(&mut self, bus: &mut impl CpuBus, take: bool) {
        if !take {
            self.fetch(bus);
        } else {
            let offset = self.fetch(bus) as i8;
            let target = self.pc.wrapping_add(offset as u16);
            self.idle6(bus, target);
            self.idle(bus);
            self.pc = target;
        }
    }

    fn branch_long(&mut self, bus: &mut impl CpuBus) {
        let offset = self.fetch16(bus) as i16;
        self.idle(bus);
        self.pc = self.pc.wrapping_add(offset as u16);
    }

    fn jump_short(&mut self, bus: &mut impl CpuBus) {
        let target = self.fetch16(bus);
        self.pc = target;
    }

    fn jump_long(&mut self, bus: &mut impl CpuBus) {
        let target = self.fetch24(bus);
        self.pc = (target & 0xFFFF) as u16;
        self.pbr = (target >> 16) as u8;
    }

    fn jump_indirect(&mut self, bus: &mut impl CpuBus) {
        let pointer = self.fetch16(bus);
        let low = self.read(bus, pointer as u32);
        let high = self.read(bus, u32::from(pointer.wrapping_add(1)));
        self.pc = u16::from_le_bytes([low, high]);
    }

    fn jump_indexed_indirect(&mut self, bus: &mut impl CpuBus) {
        let pointer = self.fetch16(bus);
        self.idle(bus);
        let address = pointer.wrapping_add(self.x);
        let low = self.read(bus, self.pc_address().wrapping_replace_bank(address));
        let high = self.read(
            bus,
            self.pc_address()
                .wrapping_replace_bank(address.wrapping_add(1)),
        );
        self.pc = u16::from_le_bytes([low, high]);
    }

    fn jump_indirect_long(&mut self, bus: &mut impl CpuBus) {
        let pointer = self.fetch16(bus);
        let low = self.read(bus, pointer as u32);
        let high = self.read(bus, u32::from(pointer.wrapping_add(1)));
        let bank = self.read(bus, u32::from(pointer.wrapping_add(2)));
        self.pc = u16::from_le_bytes([low, high]);
        self.pbr = bank;
    }

    fn call_short(&mut self, bus: &mut impl CpuBus) {
        let target = self.fetch16(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_sub(1);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.pc = target;
    }

    fn call_long(&mut self, bus: &mut impl CpuBus) {
        let low = self.fetch(bus);
        let high = self.fetch(bus);
        self.push_n(bus, self.pbr);
        self.idle(bus);
        let bank = self.fetch(bus);
        self.pc = self.pc.wrapping_sub(1);
        self.push_n(bus, (self.pc >> 8) as u8);
        self.push_n(bus, self.pc as u8);
        self.pc = u16::from_le_bytes([low, high]);
        self.pbr = bank;
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn call_indexed_indirect(&mut self, bus: &mut impl CpuBus) {
        let low = self.fetch(bus);
        self.push_n(bus, (self.pc >> 8) as u8);
        self.push_n(bus, self.pc as u8);
        let high = self.fetch(bus);
        self.idle(bus);
        let pointer = u16::from_le_bytes([low, high]).wrapping_add(self.x);
        let low = self.read(bus, self.pc_address().wrapping_replace_bank(pointer));
        let high = self.read(
            bus,
            self.pc_address()
                .wrapping_replace_bank(pointer.wrapping_add(1)),
        );
        self.pc = u16::from_le_bytes([low, high]);
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn return_interrupt(&mut self, bus: &mut impl CpuBus) {
        self.idle(bus);
        self.idle(bus);
        let p = self.pull(bus);
        self.set_p(p);
        let low = self.pull(bus);
        if self.emulation {
            let high = self.pull(bus);
            self.pc = u16::from_le_bytes([low, high]);
            self.pbr = 0;
        } else {
            let high = self.pull(bus);
            let bank = self.pull(bus);
            self.pc = u16::from_le_bytes([low, high]);
            self.pbr = bank;
        }
    }

    fn return_short(&mut self, bus: &mut impl CpuBus) {
        self.idle(bus);
        self.idle(bus);
        let low = self.pull(bus);
        let high = self.pull(bus);
        self.idle(bus);
        self.pc = u16::from_le_bytes([low, high]).wrapping_add(1);
    }

    fn return_long(&mut self, bus: &mut impl CpuBus) {
        self.idle(bus);
        self.idle(bus);
        let low = self.pull_n(bus);
        let high = self.pull_n(bus);
        let bank = self.pull_n(bus);
        self.pc = u16::from_le_bytes([low, high]).wrapping_add(1);
        self.pbr = bank;
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn push_effective_address(&mut self, bus: &mut impl CpuBus) {
        let value = self.fetch16(bus);
        self.push_n(bus, (value >> 8) as u8);
        self.push_n(bus, value as u8);
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn push_effective_indirect(&mut self, bus: &mut impl CpuBus) {
        let base = u16::from(self.fetch(bus));
        self.idle2(bus);
        let value = self.read_direct16(bus, base);
        self.push_n(bus, (value >> 8) as u8);
        self.push_n(bus, value as u8);
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn push_effective_relative(&mut self, bus: &mut impl CpuBus) {
        let offset = self.fetch16(bus) as i16;
        self.idle(bus);
        let value = self.pc.wrapping_add(offset as u16);
        self.push_n(bus, (value >> 8) as u8);
        self.push_n(bus, value as u8);
        if self.emulation {
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn push_byte(&mut self, bus: &mut impl CpuBus, value: u8) {
        self.idle(bus);
        self.push(bus, value);
    }

    fn push_word(&mut self, bus: &mut impl CpuBus, value: u16) {
        self.idle(bus);
        self.push(bus, (value >> 8) as u8);
        self.push(bus, value as u8);
    }

    fn pull_byte(&mut self, bus: &mut impl CpuBus) -> u8 {
        self.idle(bus);
        self.idle(bus);
        self.pull(bus)
    }

    fn pull_word(&mut self, bus: &mut impl CpuBus) -> u16 {
        self.idle(bus);
        self.idle(bus);
        let low = self.pull(bus);
        let high = self.pull(bus);
        u16::from_le_bytes([low, high])
    }

    fn exchange_ba(&mut self, bus: &mut impl CpuBus) {
        self.idle(bus);
        self.idle(bus);
        self.a = self.a.rotate_left(8);
        let low = self.a8();
        self.set_zn8(low);
    }

    fn exchange_ce(&mut self, bus: &mut impl CpuBus) {
        self.idle_irq(bus);
        std::mem::swap(&mut self.emulation, &mut self.flags.carry);
        if self.emulation {
            self.flags.index_8 = true;
            self.flags.mem_8 = true;
            self.x &= 0x00FF;
            self.y &= 0x00FF;
            self.s = 0x0100 | (self.s & 0x00FF);
        }
    }

    fn set_flag(&mut self, bus: &mut impl CpuBus, flag: Flag) {
        self.idle_irq(bus);
        match flag {
            Flag::Carry => self.flags.carry = true,
            Flag::IrqDisable => self.flags.irq_disable = true,
            Flag::Decimal => self.flags.decimal = true,
            Flag::Overflow => self.flags.overflow = true,
        }
    }

    fn clear_flag(&mut self, bus: &mut impl CpuBus, flag: Flag) {
        self.idle_irq(bus);
        match flag {
            Flag::Carry => self.flags.carry = false,
            Flag::IrqDisable => self.flags.irq_disable = false,
            Flag::Decimal => self.flags.decimal = false,
            Flag::Overflow => self.flags.overflow = false,
        }
    }

    fn reset_p(&mut self, bus: &mut impl CpuBus, mask: u8) {
        self.idle(bus);
        self.set_p(self.p() & !mask);
    }

    fn set_p_mask(&mut self, bus: &mut impl CpuBus, mask: u8) {
        self.idle(bus);
        self.set_p(self.p() | mask);
    }

    fn block_move(&mut self, bus: &mut impl CpuBus, adjust: i16) {
        let destination = self.fetch(bus);
        let source = self.fetch(bus);
        self.dbr = destination;
        let value = self.read(bus, (u32::from(source) << 16) | self.x as u32);
        self.write(bus, (u32::from(destination) << 16) | self.y as u32, value);
        self.idle(bus);
        if self.flags.index_8 {
            self.x = (self.x & 0xFF00) | (self.x.wrapping_add(adjust as u16) & 0x00FF);
            self.y = (self.y & 0xFF00) | (self.y.wrapping_add(adjust as u16) & 0x00FF);
        } else {
            self.x = self.x.wrapping_add(adjust as u16);
            self.y = self.y.wrapping_add(adjust as u16);
        }
        self.idle(bus);
        let old_a = self.a;
        self.a = self.a.wrapping_sub(1);
        if old_a != 0 {
            self.pc = self.pc.wrapping_sub(3);
        }
    }

    fn execute(&mut self, bus: &mut impl CpuBus, opcode: u8) {
        match opcode {
            0x00 => self.service_interrupt(bus, if self.emulation { 0xFFFE } else { 0xFFE6 }, true),
            0x01 => self.read_op(bus, ReadOp::Ora, Mode::IndexedIndirect),
            0x02 => self.service_interrupt(bus, if self.emulation { 0xFFF4 } else { 0xFFE4 }, true),
            0x03 => self.read_op(bus, ReadOp::Ora, Mode::Stack),
            0x04 => self.modify_op(bus, ModifyOp::Tsb, Mode::Direct),
            0x05 => self.read_op(bus, ReadOp::Ora, Mode::Direct),
            0x06 => self.modify_op(bus, ModifyOp::Asl, Mode::Direct),
            0x07 => self.read_op(bus, ReadOp::Ora, Mode::IndirectLong),
            0x08 => {
                let p = self.p();
                self.push_byte(bus, p);
            }
            0x09 => self.read_op(bus, ReadOp::Ora, Mode::Immediate),
            0x0A => self.modify_op(bus, ModifyOp::Asl, Mode::Implied),
            0x0B => {
                let d = self.d;
                self.idle(bus);
                self.push_n(bus, (d >> 8) as u8);
                self.push_n(bus, d as u8);
                if self.emulation {
                    self.s = 0x0100 | (self.s & 0x00FF);
                }
            }
            0x0C => self.modify_op(bus, ModifyOp::Tsb, Mode::Bank),
            0x0D => self.read_op(bus, ReadOp::Ora, Mode::Bank),
            0x0E => self.modify_op(bus, ModifyOp::Asl, Mode::Bank),
            0x0F => self.read_op(bus, ReadOp::Ora, Mode::Long),
            0x10 => self.branch(bus, !self.flags.negative),
            0x11 => self.read_op(bus, ReadOp::Ora, Mode::IndirectIndexed),
            0x12 => self.read_op(bus, ReadOp::Ora, Mode::Indirect),
            0x13 => self.read_op(bus, ReadOp::Ora, Mode::IndirectStackY),
            0x14 => self.modify_op(bus, ModifyOp::Trb, Mode::Direct),
            0x15 => self.read_op(bus, ReadOp::Ora, Mode::DirectX),
            0x16 => self.modify_op(bus, ModifyOp::Asl, Mode::DirectX),
            0x17 => self.read_op(bus, ReadOp::Ora, Mode::IndirectLongY),
            0x18 => self.clear_flag(bus, Flag::Carry),
            0x19 => self.read_op(bus, ReadOp::Ora, Mode::BankY),
            0x1A => self.modify_op(bus, ModifyOp::Inc, Mode::Implied),
            0x1B => {
                self.idle_irq(bus);
                self.s = self.a;
                if self.emulation {
                    self.s = 0x0100 | (self.s & 0x00FF);
                }
            }
            0x1C => self.modify_op(bus, ModifyOp::Trb, Mode::Bank),
            0x1D => self.read_op(bus, ReadOp::Ora, Mode::BankX),
            0x1E => self.modify_op(bus, ModifyOp::Asl, Mode::BankX),
            0x1F => self.read_op(bus, ReadOp::Ora, Mode::LongX),
            0x20 => self.call_short(bus),
            0x21 => self.read_op(bus, ReadOp::And, Mode::IndexedIndirect),
            0x22 => self.call_long(bus),
            0x23 => self.read_op(bus, ReadOp::And, Mode::Stack),
            0x24 => self.read_op(bus, ReadOp::Bit, Mode::Direct),
            0x25 => self.read_op(bus, ReadOp::And, Mode::Direct),
            0x26 => self.modify_op(bus, ModifyOp::Rol, Mode::Direct),
            0x27 => self.read_op(bus, ReadOp::And, Mode::IndirectLong),
            0x28 => {
                let p = self.pull_byte(bus);
                self.set_p(p);
            }
            0x29 => self.read_op(bus, ReadOp::And, Mode::Immediate),
            0x2A => self.modify_op(bus, ModifyOp::Rol, Mode::Implied),
            0x2B => {
                let d = self.pull_word(bus);
                self.d = d;
                self.set_zn16(d);
                if self.emulation {
                    self.s = 0x0100 | (self.s & 0x00FF);
                }
            }
            0x2C => self.read_op(bus, ReadOp::Bit, Mode::Bank),
            0x2D => self.read_op(bus, ReadOp::And, Mode::Bank),
            0x2E => self.modify_op(bus, ModifyOp::Rol, Mode::Bank),
            0x2F => self.read_op(bus, ReadOp::And, Mode::Long),
            0x30 => self.branch(bus, self.flags.negative),
            0x31 => self.read_op(bus, ReadOp::And, Mode::IndirectIndexed),
            0x32 => self.read_op(bus, ReadOp::And, Mode::Indirect),
            0x33 => self.read_op(bus, ReadOp::And, Mode::IndirectStackY),
            0x34 => self.read_op(bus, ReadOp::Bit, Mode::DirectX),
            0x35 => self.read_op(bus, ReadOp::And, Mode::DirectX),
            0x36 => self.modify_op(bus, ModifyOp::Rol, Mode::DirectX),
            0x37 => self.read_op(bus, ReadOp::And, Mode::IndirectLongY),
            0x38 => self.set_flag(bus, Flag::Carry),
            0x39 => self.read_op(bus, ReadOp::And, Mode::BankY),
            0x3A => self.modify_op(bus, ModifyOp::Dec, Mode::Implied),
            0x3B => {
                self.idle_irq(bus);
                self.a = self.s;
                self.set_zn16(self.a);
            }
            0x3C => self.read_op(bus, ReadOp::Bit, Mode::BankX),
            0x3D => self.read_op(bus, ReadOp::And, Mode::BankX),
            0x3E => self.modify_op(bus, ModifyOp::Rol, Mode::BankX),
            0x3F => self.read_op(bus, ReadOp::And, Mode::LongX),
            0x40 => self.return_interrupt(bus),
            0x41 => self.read_op(bus, ReadOp::Eor, Mode::IndexedIndirect),
            0x42 => {
                self.fetch(bus);
            }
            0x43 => self.read_op(bus, ReadOp::Eor, Mode::Stack),
            0x44 => self.block_move(bus, -1),
            0x45 => self.read_op(bus, ReadOp::Eor, Mode::Direct),
            0x46 => self.modify_op(bus, ModifyOp::Lsr, Mode::Direct),
            0x47 => self.read_op(bus, ReadOp::Eor, Mode::IndirectLong),
            0x48 => {
                let a = self.a;
                self.push_word_or_byte(bus, a);
            }
            0x49 => self.read_op(bus, ReadOp::Eor, Mode::Immediate),
            0x4A => self.modify_op(bus, ModifyOp::Lsr, Mode::Implied),
            0x4B => {
                let bank = self.pbr;
                self.push_byte(bus, bank);
            }
            0x4C => self.jump_short(bus),
            0x4D => self.read_op(bus, ReadOp::Eor, Mode::Bank),
            0x4E => self.modify_op(bus, ModifyOp::Lsr, Mode::Bank),
            0x4F => self.read_op(bus, ReadOp::Eor, Mode::Long),
            0x50 => self.branch(bus, !self.flags.overflow),
            0x51 => self.read_op(bus, ReadOp::Eor, Mode::IndirectIndexed),
            0x52 => self.read_op(bus, ReadOp::Eor, Mode::Indirect),
            0x53 => self.read_op(bus, ReadOp::Eor, Mode::IndirectStackY),
            0x54 => self.block_move(bus, 1),
            0x55 => self.read_op(bus, ReadOp::Eor, Mode::DirectX),
            0x56 => self.modify_op(bus, ModifyOp::Lsr, Mode::DirectX),
            0x57 => self.read_op(bus, ReadOp::Eor, Mode::IndirectLongY),
            0x58 => self.clear_flag(bus, Flag::IrqDisable),
            0x59 => self.read_op(bus, ReadOp::Eor, Mode::BankY),
            0x5A => {
                let y = self.y;
                self.push_index(bus, y);
            }
            0x5B => {
                self.idle_irq(bus);
                self.d = self.a;
                self.set_zn16(self.d);
            }
            0x5C => self.jump_long(bus),
            0x5D => self.read_op(bus, ReadOp::Eor, Mode::BankX),
            0x5E => self.modify_op(bus, ModifyOp::Lsr, Mode::BankX),
            0x5F => self.read_op(bus, ReadOp::Eor, Mode::LongX),
            0x60 => self.return_short(bus),
            0x61 => self.read_op(bus, ReadOp::Adc, Mode::IndexedIndirect),
            0x62 => self.push_effective_relative(bus),
            0x63 => self.read_op(bus, ReadOp::Adc, Mode::Stack),
            0x64 => self.write_op(bus, WriteOp::Stz, Mode::Direct),
            0x65 => self.read_op(bus, ReadOp::Adc, Mode::Direct),
            0x66 => self.modify_op(bus, ModifyOp::Ror, Mode::Direct),
            0x67 => self.read_op(bus, ReadOp::Adc, Mode::IndirectLong),
            0x68 => {
                let value = self.pull_word_or_byte(bus);
                self.a = value;
            }
            0x69 => self.read_op(bus, ReadOp::Adc, Mode::Immediate),
            0x6A => self.modify_op(bus, ModifyOp::Ror, Mode::Implied),
            0x6B => self.return_long(bus),
            0x6C => self.jump_indirect(bus),
            0x6D => self.read_op(bus, ReadOp::Adc, Mode::Bank),
            0x6E => self.modify_op(bus, ModifyOp::Ror, Mode::Bank),
            0x6F => self.read_op(bus, ReadOp::Adc, Mode::Long),
            0x70 => self.branch(bus, self.flags.overflow),
            0x71 => self.read_op(bus, ReadOp::Adc, Mode::IndirectIndexed),
            0x72 => self.read_op(bus, ReadOp::Adc, Mode::Indirect),
            0x73 => self.read_op(bus, ReadOp::Adc, Mode::IndirectStackY),
            0x74 => self.write_op(bus, WriteOp::Stz, Mode::DirectX),
            0x75 => self.read_op(bus, ReadOp::Adc, Mode::DirectX),
            0x76 => self.modify_op(bus, ModifyOp::Ror, Mode::DirectX),
            0x77 => self.read_op(bus, ReadOp::Adc, Mode::IndirectLongY),
            0x78 => self.set_flag(bus, Flag::IrqDisable),
            0x79 => self.read_op(bus, ReadOp::Adc, Mode::BankY),
            0x7A => {
                let value = self.pull_index(bus);
                self.y = value;
            }
            0x7B => {
                self.idle_irq(bus);
                self.a = self.d;
                self.set_zn16(self.a);
            }
            0x7C => self.jump_indexed_indirect(bus),
            0x7D => self.read_op(bus, ReadOp::Adc, Mode::BankX),
            0x7E => self.modify_op(bus, ModifyOp::Ror, Mode::BankX),
            0x7F => self.read_op(bus, ReadOp::Adc, Mode::LongX),
            0x80 => self.branch(bus, true),
            0x81 => self.write_op(bus, WriteOp::Sta, Mode::IndexedIndirect),
            0x82 => self.branch_long(bus),
            0x83 => self.write_op(bus, WriteOp::Sta, Mode::Stack),
            0x84 => self.write_op(bus, WriteOp::Sty, Mode::Direct),
            0x85 => self.write_op(bus, WriteOp::Sta, Mode::Direct),
            0x86 => self.write_op(bus, WriteOp::Stx, Mode::Direct),
            0x87 => self.write_op(bus, WriteOp::Sta, Mode::IndirectLong),
            0x88 => {
                self.idle_irq(bus);
                self.y = decrement_index(self.y, self.flags.index_8);
                self.set_zn_index(self.y, self.flags.index_8);
            }
            0x89 => self.bit_immediate(bus),
            0x8A => {
                let value = if self.flags.mem_8 {
                    u16::from(self.x as u8)
                } else {
                    self.x
                };
                self.idle_irq(bus);
                if self.flags.mem_8 {
                    self.set_a8(value as u8);
                    self.set_zn8(value as u8);
                } else {
                    self.a = value;
                    self.set_zn16(value);
                }
            }
            0x8B => {
                let bank = self.dbr;
                self.push_byte(bus, bank);
            }
            0x8C => self.write_op(bus, WriteOp::Sty, Mode::Bank),
            0x8D => self.write_op(bus, WriteOp::Sta, Mode::Bank),
            0x8E => self.write_op(bus, WriteOp::Stx, Mode::Bank),
            0x8F => self.write_op(bus, WriteOp::Sta, Mode::Long),
            0x90 => self.branch(bus, !self.flags.carry),
            0x91 => self.write_op(bus, WriteOp::Sta, Mode::IndirectIndexed),
            0x92 => self.write_op(bus, WriteOp::Sta, Mode::Indirect),
            0x93 => self.write_op(bus, WriteOp::Sta, Mode::IndirectStackY),
            0x94 => self.write_op(bus, WriteOp::Sty, Mode::DirectX),
            0x95 => self.write_op(bus, WriteOp::Sta, Mode::DirectX),
            0x96 => self.write_op(bus, WriteOp::Stx, Mode::DirectY),
            0x97 => self.write_op(bus, WriteOp::Sta, Mode::IndirectLongY),
            0x98 => {
                let value = if self.flags.mem_8 {
                    u16::from(self.y as u8)
                } else {
                    self.y
                };
                self.idle_irq(bus);
                if self.flags.mem_8 {
                    self.set_a8(value as u8);
                    self.set_zn8(value as u8);
                } else {
                    self.a = value;
                    self.set_zn16(value);
                }
            }
            0x99 => self.write_op(bus, WriteOp::Sta, Mode::BankY),
            0x9A => {
                self.idle_irq(bus);
                self.s = if self.emulation {
                    0x0100 | (self.x & 0x00FF)
                } else {
                    self.x
                };
            }
            0x9B => {
                self.idle_irq(bus);
                if self.flags.index_8 {
                    self.y = (self.y & 0xFF00) | (self.x & 0x00FF);
                    self.set_zn8(self.y as u8);
                } else {
                    self.y = self.x;
                    self.set_zn16(self.y);
                }
            }
            0x9C => self.write_op(bus, WriteOp::Stz, Mode::Bank),
            0x9D => self.write_op(bus, WriteOp::Sta, Mode::BankX),
            0x9E => self.write_op(bus, WriteOp::Stz, Mode::BankX),
            0x9F => self.write_op(bus, WriteOp::Sta, Mode::LongX),
            0xA0 => self.read_index_op(bus, IndexOp::Ldy, Mode::Immediate),
            0xA1 => self.read_op(bus, ReadOp::Lda, Mode::IndexedIndirect),
            0xA2 => self.read_index_op(bus, IndexOp::Ldx, Mode::Immediate),
            0xA3 => self.read_op(bus, ReadOp::Lda, Mode::Stack),
            0xA4 => self.read_index_op(bus, IndexOp::Ldy, Mode::Direct),
            0xA5 => self.read_op(bus, ReadOp::Lda, Mode::Direct),
            0xA6 => self.read_index_op(bus, IndexOp::Ldx, Mode::Direct),
            0xA7 => self.read_op(bus, ReadOp::Lda, Mode::IndirectLong),
            0xA8 => {
                self.idle_irq(bus);
                if self.flags.index_8 {
                    self.y = (self.y & 0xFF00) | u16::from(self.a8());
                    self.set_zn8(self.y as u8);
                } else {
                    self.y = self.a;
                    self.set_zn16(self.y);
                }
            }
            0xA9 => self.read_op(bus, ReadOp::Lda, Mode::Immediate),
            0xAA => {
                self.idle_irq(bus);
                if self.flags.index_8 {
                    self.x = (self.x & 0xFF00) | u16::from(self.a8());
                    self.set_zn8(self.x as u8);
                } else {
                    self.x = self.a;
                    self.set_zn16(self.x);
                }
            }
            0xAB => {
                let value = self.pull_byte(bus);
                self.dbr = value;
                self.set_zn8(value);
                if self.emulation {
                    self.s = 0x0100 | (self.s & 0x00FF);
                }
            }
            0xAC => self.read_index_op(bus, IndexOp::Ldy, Mode::Bank),
            0xAD => self.read_op(bus, ReadOp::Lda, Mode::Bank),
            0xAE => self.read_index_op(bus, IndexOp::Ldx, Mode::Bank),
            0xAF => self.read_op(bus, ReadOp::Lda, Mode::Long),
            0xB0 => self.branch(bus, self.flags.carry),
            0xB1 => self.read_op(bus, ReadOp::Lda, Mode::IndirectIndexed),
            0xB2 => self.read_op(bus, ReadOp::Lda, Mode::Indirect),
            0xB3 => self.read_op(bus, ReadOp::Lda, Mode::IndirectStackY),
            0xB4 => self.read_index_op(bus, IndexOp::Ldy, Mode::DirectX),
            0xB5 => self.read_op(bus, ReadOp::Lda, Mode::DirectX),
            0xB6 => self.read_index_op(bus, IndexOp::Ldx, Mode::DirectY),
            0xB7 => self.read_op(bus, ReadOp::Lda, Mode::IndirectLongY),
            0xB8 => self.clear_flag(bus, Flag::Overflow),
            0xB9 => self.read_op(bus, ReadOp::Lda, Mode::BankY),
            0xBA => {
                self.idle_irq(bus);
                let value = if self.flags.index_8 {
                    (self.x & 0xFF00) | (self.s & 0x00FF)
                } else {
                    self.s
                };
                self.x = value;
                self.set_zn_index(value, self.flags.index_8);
            }
            0xBB => {
                self.idle_irq(bus);
                if self.flags.index_8 {
                    self.x = (self.x & 0xFF00) | (self.y & 0x00FF);
                    self.set_zn8(self.x as u8);
                } else {
                    self.x = self.y;
                    self.set_zn16(self.x);
                }
            }
            0xBC => self.read_index_op(bus, IndexOp::Ldy, Mode::BankX),
            0xBD => self.read_op(bus, ReadOp::Lda, Mode::BankX),
            0xBE => self.read_index_op(bus, IndexOp::Ldx, Mode::BankY),
            0xBF => self.read_op(bus, ReadOp::Lda, Mode::LongX),
            0xC0 => self.read_index_op(bus, IndexOp::Cpy, Mode::Immediate),
            0xC1 => self.read_op(bus, ReadOp::Cmp, Mode::IndexedIndirect),
            0xC2 => {
                let mask = self.fetch(bus);
                self.reset_p(bus, mask);
            }
            0xC3 => self.read_op(bus, ReadOp::Cmp, Mode::Stack),
            0xC4 => self.read_index_op(bus, IndexOp::Cpy, Mode::Direct),
            0xC5 => self.read_op(bus, ReadOp::Cmp, Mode::Direct),
            0xC6 => self.modify_op(bus, ModifyOp::Dec, Mode::Direct),
            0xC7 => self.read_op(bus, ReadOp::Cmp, Mode::IndirectLong),
            0xC8 => {
                self.idle_irq(bus);
                self.y = increment_index(self.y, self.flags.index_8);
                self.set_zn_index(self.y, self.flags.index_8);
            }
            0xC9 => self.read_op(bus, ReadOp::Cmp, Mode::Immediate),
            0xCA => {
                self.idle_irq(bus);
                self.x = decrement_index(self.x, self.flags.index_8);
                self.set_zn_index(self.x, self.flags.index_8);
            }
            0xCB => {
                self.wai = true;
                self.idle(bus);
                self.idle(bus);
            }
            0xCC => self.read_index_op(bus, IndexOp::Cpy, Mode::Bank),
            0xCD => self.read_op(bus, ReadOp::Cmp, Mode::Bank),
            0xCE => self.modify_op(bus, ModifyOp::Dec, Mode::Bank),
            0xCF => self.read_op(bus, ReadOp::Cmp, Mode::Long),
            0xD0 => self.branch(bus, !self.flags.zero),
            0xD1 => self.read_op(bus, ReadOp::Cmp, Mode::IndirectIndexed),
            0xD2 => self.read_op(bus, ReadOp::Cmp, Mode::Indirect),
            0xD3 => self.read_op(bus, ReadOp::Cmp, Mode::IndirectStackY),
            0xD4 => self.push_effective_indirect(bus),
            0xD5 => self.read_op(bus, ReadOp::Cmp, Mode::DirectX),
            0xD6 => self.modify_op(bus, ModifyOp::Dec, Mode::DirectX),
            0xD7 => self.read_op(bus, ReadOp::Cmp, Mode::IndirectLongY),
            0xD8 => self.clear_flag(bus, Flag::Decimal),
            0xD9 => self.read_op(bus, ReadOp::Cmp, Mode::BankY),
            0xDA => {
                let x = self.x;
                self.push_index(bus, x);
            }
            0xDB => {
                self.stp = true;
                self.idle(bus);
                self.idle(bus);
            }
            0xDC => self.jump_indirect_long(bus),
            0xDD => self.read_op(bus, ReadOp::Cmp, Mode::BankX),
            0xDE => self.modify_op(bus, ModifyOp::Dec, Mode::BankX),
            0xDF => self.read_op(bus, ReadOp::Cmp, Mode::LongX),
            0xE0 => self.read_index_op(bus, IndexOp::Cpx, Mode::Immediate),
            0xE1 => self.read_op(bus, ReadOp::Sbc, Mode::IndexedIndirect),
            0xE2 => {
                let mask = self.fetch(bus);
                self.set_p_mask(bus, mask);
            }
            0xE3 => self.read_op(bus, ReadOp::Sbc, Mode::Stack),
            0xE4 => self.read_index_op(bus, IndexOp::Cpx, Mode::Direct),
            0xE5 => self.read_op(bus, ReadOp::Sbc, Mode::Direct),
            0xE6 => self.modify_op(bus, ModifyOp::Inc, Mode::Direct),
            0xE7 => self.read_op(bus, ReadOp::Sbc, Mode::IndirectLong),
            0xE8 => {
                self.idle_irq(bus);
                self.x = increment_index(self.x, self.flags.index_8);
                self.set_zn_index(self.x, self.flags.index_8);
            }
            0xE9 => self.read_op(bus, ReadOp::Sbc, Mode::Immediate),
            0xEA => self.idle_irq(bus),
            0xEB => self.exchange_ba(bus),
            0xEC => self.read_index_op(bus, IndexOp::Cpx, Mode::Bank),
            0xED => self.read_op(bus, ReadOp::Sbc, Mode::Bank),
            0xEE => self.modify_op(bus, ModifyOp::Inc, Mode::Bank),
            0xEF => self.read_op(bus, ReadOp::Sbc, Mode::Long),
            0xF0 => self.branch(bus, self.flags.zero),
            0xF1 => self.read_op(bus, ReadOp::Sbc, Mode::IndirectIndexed),
            0xF2 => self.read_op(bus, ReadOp::Sbc, Mode::Indirect),
            0xF3 => self.read_op(bus, ReadOp::Sbc, Mode::IndirectStackY),
            0xF4 => self.push_effective_address(bus),
            0xF5 => self.read_op(bus, ReadOp::Sbc, Mode::DirectX),
            0xF6 => self.modify_op(bus, ModifyOp::Inc, Mode::DirectX),
            0xF7 => self.read_op(bus, ReadOp::Sbc, Mode::IndirectLongY),
            0xF8 => self.set_flag(bus, Flag::Decimal),
            0xF9 => self.read_op(bus, ReadOp::Sbc, Mode::BankY),
            0xFA => {
                let value = self.pull_index(bus);
                self.x = value;
            }
            0xFB => self.exchange_ce(bus),
            0xFC => self.call_indexed_indirect(bus),
            0xFD => self.read_op(bus, ReadOp::Sbc, Mode::BankX),
            0xFE => self.modify_op(bus, ModifyOp::Inc, Mode::BankX),
            0xFF => self.read_op(bus, ReadOp::Sbc, Mode::LongX),
        }
    }

    fn push_word_or_byte(&mut self, bus: &mut impl CpuBus, value: u16) {
        if self.flags.mem_8 {
            self.push_byte(bus, value as u8);
        } else {
            self.push_word(bus, value);
        }
    }

    fn pull_word_or_byte(&mut self, bus: &mut impl CpuBus) -> u16 {
        if self.flags.mem_8 {
            let value = self.pull_byte(bus);
            self.a = (self.a & 0xFF00) | u16::from(value);
            self.set_zn8(value);
            self.a
        } else {
            let value = self.pull_word(bus);
            self.a = value;
            self.set_zn16(value);
            value
        }
    }

    fn push_index(&mut self, bus: &mut impl CpuBus, value: u16) {
        if self.flags.index_8 {
            self.push_byte(bus, value as u8);
        } else {
            self.push_word(bus, value);
        }
    }

    fn pull_index(&mut self, bus: &mut impl CpuBus) -> u16 {
        if self.flags.index_8 {
            let value = self.pull_byte(bus);
            self.set_zn8(value);
            u16::from(value)
        } else {
            let value = self.pull_word(bus);
            self.set_zn16(value);
            value
        }
    }

    fn set_zn_index(&mut self, value: u16, eight: bool) {
        if eight {
            self.set_zn8(value as u8);
        } else {
            self.set_zn16(value);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadOp {
    Ora,
    And,
    Eor,
    Adc,
    Lda,
    Cmp,
    Sbc,
    Bit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexOp {
    Ldx,
    Ldy,
    Cpx,
    Cpy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifyOp {
    Asl,
    Lsr,
    Rol,
    Ror,
    Inc,
    Dec,
    Tsb,
    Trb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteOp {
    Sta,
    Stz,
    Stx,
    Sty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Implied,
    Immediate,
    Direct,
    DirectX,
    DirectY,
    Bank,
    BankX,
    BankY,
    Long,
    LongX,
    Indirect,
    IndexedIndirect,
    IndirectIndexed,
    IndirectLong,
    IndirectLongY,
    Stack,
    IndirectStackY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flag {
    Carry,
    IrqDisable,
    Decimal,
    Overflow,
}

fn increment_index(value: u16, eight: bool) -> u16 {
    if eight {
        (value & 0xFF00) | (value.wrapping_add(1) & 0x00FF)
    } else {
        value.wrapping_add(1)
    }
}

fn decrement_index(value: u16, eight: bool) -> u16 {
    if eight {
        (value & 0xFF00) | (value.wrapping_sub(1) & 0x00FF)
    } else {
        value.wrapping_sub(1)
    }
}

trait AddressExt {
    fn wrapping_replace_bank(self, offset: u16) -> u32;
}

impl AddressExt for u32 {
    fn wrapping_replace_bank(self, offset: u16) -> u32 {
        (self & 0x00FF_0000) | u32::from(offset)
    }
}

impl fmt::Display for Cpu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PC={:02X}:{:04X} A={:04X} X={:04X} Y={:04X} S={:04X} D={:04X} DB={:02X} P={:02X} E={}",
            self.pbr,
            self.pc,
            self.a,
            self.x,
            self.y,
            self.s,
            self.d,
            self.dbr,
            self.p(),
            u8::from(self.emulation)
        )
    }
}
