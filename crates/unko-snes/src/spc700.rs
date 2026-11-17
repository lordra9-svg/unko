//! Cycle-stepped Sony SPC700 (S-SMP) CPU.
//!
//! The interpreter executes one instruction per [`Spc700::step`] and performs
//! every bus access through [`SpcBus`], so the 64 KiB APU address space sees the
//! documented number and order of cycles. Behaviour follows the public SPC700
//! documentation and the ISC-licensed ares `spc700` implementation. The MIT
//! `spc700` crate is used only as a cross-check oracle in tests.

use std::fmt;

/// The 64 KiB SPC700 address space.
pub trait SpcBus {
    /// Read one byte and advance one cycle.
    fn read(&mut self, address: u16) -> u8;
    /// Write one byte and advance one cycle.
    fn write(&mut self, address: u16, value: u8);
    /// Advance one idle cycle.
    fn idle(&mut self) {}
    /// Advance the S-SMP timers by `clocks` 2.050560 MHz clocks.
    fn advance(&mut self, _clocks: u32) {}
    /// Advance the cooperative peripherals (the S-DSP) by `clocks` 2.050560 MHz
    /// clocks. ares `SMP::wait` steps the thread and then synchronizes the DSP,
    /// so this runs *before* the access it belongs to.
    fn step_clock(&mut self, _clocks: u32) {}

    /// Access cost in S-SMP time units (ares `cycleWaitStates` / 2).
    ///
    /// One unit is two APU clocks: the hardware SMP thread runs at 2.050560 MHz
    /// and an opcode cycle costs two clocks by default. `$F0` bits 4-7 select
    /// the internal/external wait states, including the glitchy 10/20 values.
    fn wait_cycles(&mut self, _address: Option<u16>) -> u32 {
        1
    }

    /// Clocks the timers advance for an access (ares `timerWaitStates`).
    ///
    /// The timers are *not* affected by the 8/16-clock glitch: ares steps them
    /// by 2/4/8/16 clocks, not 2/4/10/20.
    fn timer_cycles(&mut self, _address: Option<u16>) -> u32 {
        2
    }

    /// Whether an access splits its wait around the data fetch. ares reads of
    /// the `$F4-$F7` ports call `wait(halve)` twice.
    fn split_port_read(&mut self, _address: u16) -> bool {
        false
    }
}

/// SPC700 processor status flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// Carry.
    pub carry: bool,
    /// Zero.
    pub zero: bool,
    /// Interrupt disable.
    pub irq_disable: bool,
    /// Half-carry.
    pub half_carry: bool,
    /// Break.
    pub break_flag: bool,
    /// Direct-page selector.
    pub page: bool,
    /// Overflow.
    pub overflow: bool,
    /// Negative.
    pub negative: bool,
}

impl Flags {
    /// Pack the flags into a PSW byte.
    pub const fn to_byte(self) -> u8 {
        (self.carry as u8)
            | ((self.zero as u8) << 1)
            | ((self.irq_disable as u8) << 2)
            | ((self.half_carry as u8) << 3)
            | ((self.break_flag as u8) << 4)
            | ((self.page as u8) << 5)
            | ((self.overflow as u8) << 6)
            | ((self.negative as u8) << 7)
    }

    /// Unpack a PSW byte.
    pub const fn from_byte(value: u8) -> Self {
        Self {
            carry: value & 0x01 != 0,
            zero: value & 0x02 != 0,
            irq_disable: value & 0x04 != 0,
            half_carry: value & 0x08 != 0,
            break_flag: value & 0x10 != 0,
            page: value & 0x20 != 0,
            overflow: value & 0x40 != 0,
            negative: value & 0x80 != 0,
        }
    }
}

/// The S-SMP CPU register file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spc700 {
    /// Accumulator.
    pub a: u8,
    /// X index.
    pub x: u8,
    /// Y index.
    pub y: u8,
    /// Stack pointer.
    pub sp: u8,
    /// Program counter.
    pub pc: u16,
    /// Processor status flags.
    pub flags: Flags,
    /// Waiting for an interrupt (`WAIT`).
    pub wait: bool,
    /// Stopped (`STOP`).
    pub stop: bool,
    /// Total S-SMP clocks retired.
    pub cycles: u64,
}

impl Default for Spc700 {
    fn default() -> Self {
        Self::new()
    }
}

impl Spc700 {
    /// Create a CPU in its power-on state.
    pub const fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xEF,
            pc: 0x0200,
            flags: Flags {
                carry: false,
                zero: false,
                irq_disable: false,
                half_carry: false,
                break_flag: false,
                page: false,
                overflow: false,
                negative: false,
            },
            wait: false,
            stop: false,
            cycles: 0,
        }
    }

    /// Reset the CPU.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Pack the PSW.
    pub const fn psw(&self) -> u8 {
        self.flags.to_byte()
    }

    /// Load the PSW.
    pub fn set_psw(&mut self, value: u8) {
        self.flags = Flags::from_byte(value);
    }

    /// Wake the CPU from `WAIT`/`STOP`.
    pub fn wake(&mut self) {
        self.wait = false;
        self.stop = false;
    }

    /// Execute one instruction. Returns the number of bus cycles retired.
    pub fn step(&mut self, bus: &mut impl SpcBus) -> u32 {
        let start = self.cycles;
        if self.wait || self.stop {
            self.idle(bus);
            return (self.cycles - start) as u32;
        }
        let opcode = self.fetch(bus);
        self.execute(bus, opcode);
        (self.cycles - start) as u32
    }

    fn read(&mut self, bus: &mut impl SpcBus, address: u16) -> u8 {
        let wait = bus.wait_cycles(Some(address));
        self.cycles += u64::from(wait);
        let clocks = wait * 2;
        if bus.split_port_read(address) {
            // ares `SMP::read`: $F4-$F7 split the wait into two halves around
            // the data fetch, halving both the CPU cost and the timer step.
            let half = bus.timer_cycles(Some(address)) / 2;
            bus.step_clock(clocks / 2);
            bus.advance(half);
            let value = bus.read(address);
            bus.step_clock(clocks / 2);
            bus.advance(half);
            return value;
        }
        // ares steps the thread and the timers during the wait, before the
        // access, so a DSP register read observes the post-step state.
        let timer = bus.timer_cycles(Some(address));
        bus.step_clock(clocks);
        bus.advance(timer);
        bus.read(address)
    }

    fn write(&mut self, bus: &mut impl SpcBus, address: u16, value: u8) {
        let wait = bus.wait_cycles(Some(address));
        self.cycles += u64::from(wait);
        let timer = bus.timer_cycles(Some(address));
        bus.step_clock(wait * 2);
        bus.advance(timer);
        bus.write(address, value);
    }

    fn idle(&mut self, bus: &mut impl SpcBus) {
        let wait = bus.wait_cycles(None);
        self.cycles += u64::from(wait);
        let timer = bus.timer_cycles(None);
        bus.step_clock(wait * 2);
        bus.advance(timer);
        bus.idle();
    }

    /// The dummy `read(PC)` cycle ares uses for implied instructions: it is a
    /// real access (external wait-state selector), not an internal idle.
    fn dummy_read(&mut self, bus: &mut impl SpcBus) {
        let address = self.pc;
        self.read(bus, address);
    }

    fn fetch(&mut self, bus: &mut impl SpcBus) -> u8 {
        let value = self.read(bus, self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn fetch16(&mut self, bus: &mut impl SpcBus) -> u16 {
        let low = self.fetch(bus);
        let high = self.fetch(bus);
        u16::from_le_bytes([low, high])
    }

    fn load(&mut self, bus: &mut impl SpcBus, address: u8) -> u8 {
        let full = (u16::from(self.flags.page) << 8) | u16::from(address);
        self.read(bus, full)
    }

    fn store(&mut self, bus: &mut impl SpcBus, address: u8, value: u8) {
        let full = (u16::from(self.flags.page) << 8) | u16::from(address);
        self.write(bus, full, value);
    }

    fn load16(&mut self, bus: &mut impl SpcBus, address: u8) -> u16 {
        let low = self.load(bus, address);
        let high = self.load(bus, address.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn push(&mut self, bus: &mut impl SpcBus, value: u8) {
        self.write(bus, 0x0100 | u16::from(self.sp), value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pull(&mut self, bus: &mut impl SpcBus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read(bus, 0x0100 | u16::from(self.sp))
    }

    fn set_zn(&mut self, value: u8) {
        self.flags.zero = value == 0;
        self.flags.negative = value & 0x80 != 0;
    }

    fn set_zn16(&mut self, value: u16) {
        self.flags.zero = value == 0;
        self.flags.negative = value & 0x8000 != 0;
    }

    /// YA as a 16-bit value, with Y as the high byte.
    pub fn ya(&self) -> u16 {
        (u16::from(self.y) << 8) | u16::from(self.a)
    }

    /// Load YA from a 16-bit value.
    pub fn set_ya(&mut self, value: u16) {
        self.a = value as u8;
        self.y = (value >> 8) as u8;
    }

    fn adc(&mut self, left: u8, right: u8) -> u8 {
        let result = u16::from(left) + u16::from(right) + u16::from(self.flags.carry);
        self.flags.carry = result > 0xFF;
        self.flags.zero = result as u8 == 0;
        self.flags.half_carry = (left ^ right ^ result as u8) & 0x10 != 0;
        self.flags.overflow = (!(left ^ right) & (left ^ result as u8) & 0x80) != 0;
        self.flags.negative = result as u8 & 0x80 != 0;
        result as u8
    }

    fn sbc(&mut self, left: u8, right: u8) -> u8 {
        self.adc(left, !right)
    }

    fn and(&mut self, left: u8, right: u8) -> u8 {
        let result = left & right;
        self.set_zn(result);
        result
    }

    fn or(&mut self, left: u8, right: u8) -> u8 {
        let result = left | right;
        self.set_zn(result);
        result
    }

    fn eor(&mut self, left: u8, right: u8) -> u8 {
        let result = left ^ right;
        self.set_zn(result);
        result
    }

    fn cmp(&mut self, left: u8, right: u8) -> u8 {
        let result = i16::from(left) - i16::from(right);
        self.flags.carry = result >= 0;
        self.flags.zero = result as u8 == 0;
        self.flags.negative = result as u8 & 0x80 != 0;
        left
    }

    fn cpw(&mut self, left: u16, right: u16) -> u16 {
        let result = i32::from(left) - i32::from(right);
        self.flags.carry = result >= 0;
        self.flags.zero = result as u16 == 0;
        self.flags.negative = result as u16 & 0x8000 != 0;
        left
    }

    fn ld(&mut self, _left: u8, right: u8) -> u8 {
        self.set_zn(right);
        right
    }

    fn ldw(&mut self, _left: u16, right: u16) -> u16 {
        self.set_zn16(right);
        right
    }

    fn adw(&mut self, left: u16, right: u16) -> u16 {
        self.flags.carry = false;
        let low = self.adc(left as u8, right as u8);
        let high = self.adc((left >> 8) as u8, (right >> 8) as u8);
        let result = u16::from_le_bytes([low, high]);
        self.flags.zero = result == 0;
        result
    }

    fn sbw(&mut self, left: u16, right: u16) -> u16 {
        self.flags.carry = true;
        let low = self.sbc(left as u8, right as u8);
        let high = self.sbc((left >> 8) as u8, (right >> 8) as u8);
        let result = u16::from_le_bytes([low, high]);
        self.flags.zero = result == 0;
        result
    }

    fn asl(&mut self, value: u8) -> u8 {
        self.flags.carry = value & 0x80 != 0;
        let result = value << 1;
        self.set_zn(result);
        result
    }

    fn lsr(&mut self, value: u8) -> u8 {
        self.flags.carry = value & 0x01 != 0;
        let result = value >> 1;
        self.set_zn(result);
        result
    }

    fn rol(&mut self, value: u8) -> u8 {
        let carry = self.flags.carry;
        self.flags.carry = value & 0x80 != 0;
        let result = (value << 1) | u8::from(carry);
        self.set_zn(result);
        result
    }

    fn ror(&mut self, value: u8) -> u8 {
        let carry = self.flags.carry;
        self.flags.carry = value & 0x01 != 0;
        let result = (u8::from(carry) << 7) | (value >> 1);
        self.set_zn(result);
        result
    }

    fn inc(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.set_zn(result);
        result
    }

    fn dec(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.set_zn(result);
        result
    }

    fn branch(&mut self, bus: &mut impl SpcBus, take: bool) {
        let displacement = self.fetch(bus) as i8;
        if !take {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }

    fn execute(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        match opcode {
            0x00 => self.dummy_read(bus),
            0x01 => self.call_table(bus, 0),
            0x02 => self.absolute_bit_set(bus, 0, true),
            0x03 => self.branch_bit(bus, 0, true),
            0x04 => self.direct_read(bus, 0x04),
            0x05 => self.absolute_read(bus, 0x05),
            0x06 => self.indirect_x_read(bus, 0x06),
            0x07 => self.indexed_indirect_read(bus, 0x07),
            0x08 => self.immediate_read(bus, 0x08),
            0x09 => self.direct_direct_modify(bus, 0x09),
            0x0a => self.absolute_bit_modify(bus, 0),
            0x0b => self.direct_modify(bus, 0x0B),
            0x0c => self.absolute_modify(bus, 0x0C),
            0x0d => self.push_reg(bus, Register::P),
            0x0e => self.test_set_bits_absolute(bus, true),
            0x0f => self.break_interrupt(bus),
            0x10 => self.branch(bus, !self.flags.negative),
            0x11 => self.call_table(bus, 1),
            0x12 => self.absolute_bit_set(bus, 0, false),
            0x13 => self.branch_bit(bus, 0, false),
            0x14 => self.direct_indexed_read(bus, 0x14, Register::X),
            0x15 => self.absolute_indexed_read(bus, 0x15, Register::X),
            0x16 => self.absolute_indexed_read(bus, 0x16, Register::Y),
            0x17 => self.indirect_indexed_read(bus, 0x17, Register::Y),
            0x18 => self.direct_immediate_modify(bus, 0x18),
            0x19 => self.indirect_x_write_indirect_y(bus, 0x19),
            0x1a => self.direct_modify_word(bus, -1),
            0x1b => self.direct_indexed_modify(bus, 0x1B, Register::X),
            0x1c => self.implied_modify(bus, 0x1C, Register::A),
            0x1d => self.implied_modify(bus, 0x1D, Register::X),
            0x1e => self.absolute_read(bus, 0x1E),
            0x1f => self.jump_indirect_x(bus),
            0x20 => self.flag_set(bus, Flag::Page, false),
            0x21 => self.call_table(bus, 2),
            0x22 => self.absolute_bit_set(bus, 1, true),
            0x23 => self.branch_bit(bus, 1, true),
            0x24 => self.direct_read(bus, 0x24),
            0x25 => self.absolute_read(bus, 0x25),
            0x26 => self.indirect_x_read(bus, 0x26),
            0x27 => self.indexed_indirect_read(bus, 0x27),
            0x28 => self.immediate_read(bus, 0x28),
            0x29 => self.direct_direct_modify(bus, 0x29),
            0x2a => self.absolute_bit_modify(bus, 1),
            0x2b => self.direct_modify(bus, 0x2B),
            0x2c => self.absolute_modify(bus, 0x2C),
            0x2d => self.push_reg(bus, Register::A),
            0x2e => self.branch_not_direct(bus),
            0x2f => self.branch(bus, true),
            0x30 => self.branch(bus, self.flags.negative),
            0x31 => self.call_table(bus, 3),
            0x32 => self.absolute_bit_set(bus, 1, false),
            0x33 => self.branch_bit(bus, 1, false),
            0x34 => self.direct_indexed_read(bus, 0x34, Register::X),
            0x35 => self.absolute_indexed_read(bus, 0x35, Register::X),
            0x36 => self.absolute_indexed_read(bus, 0x36, Register::Y),
            0x37 => self.indirect_indexed_read(bus, 0x37, Register::Y),
            0x38 => self.direct_immediate_modify(bus, 0x38),
            0x39 => self.indirect_x_write_indirect_y(bus, 0x39),
            0x3a => self.direct_modify_word(bus, 1),
            0x3b => self.direct_indexed_modify(bus, 0x3B, Register::X),
            0x3c => self.implied_modify(bus, 0x3C, Register::A),
            0x3d => self.implied_modify(bus, 0x3D, Register::X),
            0x3e => self.direct_read(bus, 0x3E),
            0x3f => self.call_absolute(bus),
            0x40 => self.flag_set(bus, Flag::Page, true),
            0x41 => self.call_table(bus, 4),
            0x42 => self.absolute_bit_set(bus, 2, true),
            0x43 => self.branch_bit(bus, 2, true),
            0x44 => self.direct_read(bus, 0x44),
            0x45 => self.absolute_read(bus, 0x45),
            0x46 => self.indirect_x_read(bus, 0x46),
            0x47 => self.indexed_indirect_read(bus, 0x47),
            0x48 => self.immediate_read(bus, 0x48),
            0x49 => self.direct_direct_modify(bus, 0x49),
            0x4a => self.absolute_bit_modify(bus, 2),
            0x4b => self.direct_modify(bus, 0x4B),
            0x4c => self.absolute_modify(bus, 0x4C),
            0x4d => self.push_reg(bus, Register::X),
            0x4e => self.test_set_bits_absolute(bus, false),
            0x4f => self.call_page(bus),
            0x50 => self.branch(bus, !self.flags.overflow),
            0x51 => self.call_table(bus, 5),
            0x52 => self.absolute_bit_set(bus, 2, false),
            0x53 => self.branch_bit(bus, 2, false),
            0x54 => self.direct_indexed_read(bus, 0x54, Register::X),
            0x55 => self.absolute_indexed_read(bus, 0x55, Register::X),
            0x56 => self.absolute_indexed_read(bus, 0x56, Register::Y),
            0x57 => self.indirect_indexed_read(bus, 0x57, Register::Y),
            0x58 => self.direct_immediate_modify(bus, 0x58),
            0x59 => self.indirect_x_write_indirect_y(bus, 0x59),
            0x5a => self.direct_compare_word(bus),
            0x5b => self.direct_indexed_modify(bus, 0x5B, Register::X),
            0x5c => self.implied_modify(bus, 0x5C, Register::A),
            0x5d => self.transfer(bus, Register::A, Register::X),
            0x5e => self.absolute_read(bus, 0x5E),
            0x5f => self.jump_absolute(bus),
            0x60 => self.flag_set(bus, Flag::Carry, false),
            0x61 => self.call_table(bus, 6),
            0x62 => self.absolute_bit_set(bus, 3, true),
            0x63 => self.branch_bit(bus, 3, true),
            0x64 => self.direct_read(bus, 0x64),
            0x65 => self.absolute_read(bus, 0x65),
            0x66 => self.indirect_x_read(bus, 0x66),
            0x67 => self.indexed_indirect_read(bus, 0x67),
            0x68 => self.immediate_read(bus, 0x68),
            0x69 => self.direct_direct_compare(bus, 0x69),
            0x6a => self.absolute_bit_modify(bus, 3),
            0x6b => self.direct_modify(bus, 0x6B),
            0x6c => self.absolute_modify(bus, 0x6C),
            0x6d => self.push_reg(bus, Register::Y),
            0x6e => self.branch_not_direct_decrement(bus),
            0x6f => self.return_subroutine(bus),
            0x70 => self.branch(bus, self.flags.overflow),
            0x71 => self.call_table(bus, 7),
            0x72 => self.absolute_bit_set(bus, 3, false),
            0x73 => self.branch_bit(bus, 3, false),
            0x74 => self.direct_indexed_read(bus, 0x74, Register::X),
            0x75 => self.absolute_indexed_read(bus, 0x75, Register::X),
            0x76 => self.absolute_indexed_read(bus, 0x76, Register::Y),
            0x77 => self.indirect_indexed_read(bus, 0x77, Register::Y),
            0x78 => self.direct_immediate_compare(bus, 0x78),
            0x79 => self.indirect_x_compare_indirect_y(bus, 0x79),
            0x7a => self.direct_read_word(bus, 0x7A),
            0x7b => self.direct_indexed_modify(bus, 0x7B, Register::X),
            0x7c => self.implied_modify(bus, 0x7C, Register::A),
            0x7d => self.transfer(bus, Register::X, Register::A),
            0x7e => self.direct_read(bus, 0x7E),
            0x7f => self.return_interrupt(bus),
            0x80 => self.flag_set(bus, Flag::Carry, true),
            0x81 => self.call_table(bus, 8),
            0x82 => self.absolute_bit_set(bus, 4, true),
            0x83 => self.branch_bit(bus, 4, true),
            0x84 => self.direct_read(bus, 0x84),
            0x85 => self.absolute_read(bus, 0x85),
            0x86 => self.indirect_x_read(bus, 0x86),
            0x87 => self.indexed_indirect_read(bus, 0x87),
            0x88 => self.immediate_read(bus, 0x88),
            0x89 => self.direct_direct_modify(bus, 0x89),
            0x8a => self.absolute_bit_modify(bus, 4),
            0x8b => self.direct_modify(bus, 0x8B),
            0x8c => self.absolute_modify(bus, 0x8C),
            0x8d => self.immediate_read_register(bus, Register::Y),
            0x8e => self.pull_p(bus),
            0x8f => self.direct_immediate_write(bus),
            0x90 => self.branch(bus, !self.flags.carry),
            0x91 => self.call_table(bus, 9),
            0x92 => self.absolute_bit_set(bus, 4, false),
            0x93 => self.branch_bit(bus, 4, false),
            0x94 => self.direct_indexed_read(bus, 0x94, Register::X),
            0x95 => self.absolute_indexed_read(bus, 0x95, Register::X),
            0x96 => self.absolute_indexed_read(bus, 0x96, Register::Y),
            0x97 => self.indirect_indexed_read(bus, 0x97, Register::Y),
            0x98 => self.direct_immediate_modify(bus, 0x98),
            0x99 => self.indirect_x_write_indirect_y(bus, 0x99),
            0x9a => self.direct_read_word(bus, 0x9A),
            0x9b => self.direct_indexed_modify(bus, 0x9B, Register::X),
            0x9c => self.implied_modify(bus, 0x9C, Register::A),
            0x9d => self.transfer(bus, Register::SP, Register::X),
            0x9e => self.divide(bus),
            0x9f => self.exchange_nibble(bus),
            0xa0 => self.flag_set(bus, Flag::InterruptDisable, true),
            0xa1 => self.call_table(bus, 10),
            0xa2 => self.absolute_bit_set(bus, 5, true),
            0xa3 => self.branch_bit(bus, 5, true),
            0xa4 => self.direct_read(bus, 0xA4),
            0xa5 => self.absolute_read(bus, 0xA5),
            0xa6 => self.indirect_x_read(bus, 0xA6),
            0xa7 => self.indexed_indirect_read(bus, 0xA7),
            0xa8 => self.immediate_read(bus, 0xA8),
            0xa9 => self.direct_direct_modify(bus, 0xA9),
            0xaa => self.absolute_bit_modify(bus, 5),
            0xab => self.direct_modify(bus, 0xAB),
            0xac => self.absolute_modify(bus, 0xAC),
            0xad => self.immediate_compare_register(bus, Register::Y),
            0xae => self.pull_register(bus, Register::A),
            0xaf => self.indirect_x_increment_write(bus, Register::A),
            0xb0 => self.branch(bus, self.flags.carry),
            0xb1 => self.call_table(bus, 11),
            0xb2 => self.absolute_bit_set(bus, 5, false),
            0xb3 => self.branch_bit(bus, 5, false),
            0xb4 => self.direct_indexed_read(bus, 0xB4, Register::X),
            0xb5 => self.absolute_indexed_read(bus, 0xB5, Register::X),
            0xb6 => self.absolute_indexed_read(bus, 0xB6, Register::Y),
            0xb7 => self.indirect_indexed_read(bus, 0xB7, Register::Y),
            0xb8 => self.direct_immediate_modify(bus, 0xB8),
            0xb9 => self.indirect_x_write_indirect_y(bus, 0xB9),
            0xba => self.direct_read_word(bus, 0xBA),
            0xbb => self.direct_indexed_modify(bus, 0xBB, Register::X),
            0xbc => self.implied_modify(bus, 0xBC, Register::A),
            0xbd => self.transfer(bus, Register::X, Register::SP),
            0xbe => self.decimal_adjust_sub(bus),
            0xbf => self.indirect_x_increment_read(bus, Register::A),
            0xc0 => self.flag_set(bus, Flag::InterruptDisable, false),
            0xc1 => self.call_table(bus, 12),
            0xc2 => self.absolute_bit_set(bus, 6, true),
            0xc3 => self.branch_bit(bus, 6, true),
            0xc4 => self.direct_write(bus, Register::A),
            0xc5 => self.absolute_write(bus, Register::A),
            0xc6 => self.indirect_x_write(bus, Register::A),
            0xc7 => self.indexed_indirect_write(bus, Register::A, Register::X),
            0xc8 => self.immediate_compare_register(bus, Register::X),
            0xc9 => self.absolute_write(bus, Register::X),
            0xca => self.absolute_bit_modify(bus, 6),
            0xcb => self.direct_write(bus, Register::Y),
            0xcc => self.absolute_write(bus, Register::Y),
            0xcd => self.immediate_read_register(bus, Register::X),
            0xce => self.pull_register(bus, Register::X),
            0xcf => self.multiply(bus),
            0xd0 => self.branch(bus, !self.flags.zero),
            0xd1 => self.call_table(bus, 13),
            0xd2 => self.absolute_bit_set(bus, 6, false),
            0xd3 => self.branch_bit(bus, 6, false),
            0xd4 => self.direct_indexed_write(bus, Register::A, Register::X),
            0xd5 => self.absolute_indexed_write(bus, Register::X),
            0xd6 => self.absolute_indexed_write(bus, Register::Y),
            0xd7 => self.indirect_indexed_write(bus, Register::A, Register::Y),
            0xd8 => self.direct_write(bus, Register::X),
            0xd9 => self.direct_indexed_write(bus, Register::X, Register::Y),
            0xda => self.direct_write_word(bus),
            0xdb => self.direct_indexed_write(bus, Register::Y, Register::X),
            0xdc => self.implied_modify(bus, 0xDC, Register::Y),
            0xdd => self.transfer(bus, Register::Y, Register::A),
            0xde => self.branch_not_direct_indexed(bus, Register::X),
            0xdf => self.decimal_adjust_add(bus),
            0xe0 => self.overflow_clear(bus),
            0xe1 => self.call_table(bus, 14),
            0xe2 => self.absolute_bit_set(bus, 7, true),
            0xe3 => self.branch_bit(bus, 7, true),
            0xe4 => self.direct_read(bus, 0xE4),
            0xe5 => self.absolute_read(bus, 0xE5),
            0xe6 => self.indirect_x_read(bus, 0xE6),
            0xe7 => self.indexed_indirect_read(bus, 0xE7),
            0xe8 => self.immediate_read(bus, 0xE8),
            0xe9 => self.absolute_read(bus, 0xE9),
            0xea => self.absolute_bit_modify(bus, 7),
            0xeb => self.direct_read(bus, 0xEB),
            0xec => self.absolute_read(bus, 0xEC),
            0xed => self.complement_carry(bus),
            0xee => self.pull_register(bus, Register::Y),
            0xef => self.wait_instruction(bus),
            0xf0 => self.branch(bus, self.flags.zero),
            0xf1 => self.call_table(bus, 15),
            0xf2 => self.absolute_bit_set(bus, 7, false),
            0xf3 => self.branch_bit(bus, 7, false),
            0xf4 => self.direct_indexed_read(bus, 0xF4, Register::X),
            0xf5 => self.absolute_indexed_read(bus, 0xF5, Register::X),
            0xf6 => self.absolute_indexed_read(bus, 0xF6, Register::Y),
            0xf7 => self.indirect_indexed_read(bus, 0xF7, Register::Y),
            0xf8 => self.direct_read(bus, 0xF8),
            0xf9 => self.direct_indexed_read(bus, 0xF9, Register::Y),
            0xfa => self.direct_direct_write(bus),
            0xfb => self.direct_indexed_read(bus, 0xFB, Register::X),
            0xfc => self.implied_modify(bus, 0xFC, Register::Y),
            0xfd => self.transfer(bus, Register::A, Register::Y),
            0xfe => self.branch_not_y_decrement(bus),
            0xff => self.stop_instruction(bus),
        }
    }

    fn direct_read(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        self.apply_read(bus, opcode, value);
    }

    fn direct_indexed_read(&mut self, bus: &mut impl SpcBus, opcode: u8, register: Register) {
        let address = self.fetch(bus);
        self.idle(bus);
        let value = self.load(bus, address.wrapping_add(self.register(register)));
        self.apply_read(bus, opcode, value);
    }

    fn absolute_read(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let address = self.fetch16(bus);
        let value = self.read(bus, address);
        self.apply_read(bus, opcode, value);
    }

    fn absolute_indexed_read(&mut self, bus: &mut impl SpcBus, opcode: u8, register: Register) {
        let address = self.fetch16(bus);
        self.idle(bus);
        let value = self.read(
            bus,
            address.wrapping_add(u16::from(self.register(register))),
        );
        self.apply_read(bus, opcode, value);
    }

    fn immediate_read(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let value = self.fetch(bus);
        self.apply_read(bus, opcode, value);
    }

    fn immediate_read_register(&mut self, bus: &mut impl SpcBus, register: Register) {
        let value = self.fetch(bus);
        let result = self.ld(0, value);
        self.set_register(register, result);
    }

    fn immediate_compare_register(&mut self, bus: &mut impl SpcBus, register: Register) {
        let value = self.fetch(bus);
        match register {
            Register::X => {
                let _ = self.cmp(self.x, value);
            }
            Register::Y => {
                let _ = self.cmp(self.y, value);
            }
            _ => {}
        }
    }

    fn indirect_x_read(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        self.idle(bus);
        let value = self.load(bus, self.x);
        self.apply_read(bus, opcode, value);
    }

    fn indexed_indirect_read(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let indirect = self.fetch(bus);
        self.idle(bus);
        let pointer = self.load16(bus, indirect.wrapping_add(self.x));
        let value = self.read(bus, pointer);
        self.apply_read(bus, opcode, value);
    }

    fn indirect_indexed_read(&mut self, bus: &mut impl SpcBus, opcode: u8, register: Register) {
        let indirect = self.fetch(bus);
        self.idle(bus);
        let pointer = self.load16(bus, indirect);
        let value = self.read(
            bus,
            pointer.wrapping_add(u16::from(self.register(register))),
        );
        self.apply_read(bus, opcode, value);
    }

    fn apply_read(&mut self, _bus: &mut impl SpcBus, opcode: u8, value: u8) {
        match opcode {
            0x04 | 0x05 | 0x06 | 0x07 | 0x08 | 0x14 | 0x15 | 0x16 | 0x17 => {
                self.a = self.or(self.a, value);
            }
            0x24 | 0x25 | 0x26 | 0x27 | 0x28 | 0x34 | 0x35 | 0x36 | 0x37 => {
                self.a = self.and(self.a, value);
            }
            0x44 | 0x45 | 0x46 | 0x47 | 0x48 | 0x54 | 0x55 | 0x56 | 0x57 => {
                self.a = self.eor(self.a, value);
            }
            0x64 | 0x65 | 0x66 | 0x67 | 0x68 | 0x74 | 0x75 | 0x76 | 0x77 => {
                let _ = self.cmp(self.a, value);
            }
            0x84 | 0x85 | 0x86 | 0x87 | 0x88 | 0x94 | 0x95 | 0x96 | 0x97 => {
                self.a = self.adc(self.a, value);
            }
            0xA4 | 0xA5 | 0xA6 | 0xA7 | 0xA8 | 0xB4 | 0xB5 | 0xB6 | 0xB7 => {
                self.a = self.sbc(self.a, value);
            }
            0xE4 | 0xE5 | 0xE6 | 0xE7 | 0xE8 | 0xF4 | 0xF5 | 0xF6 | 0xF7 => {
                self.a = self.ld(self.a, value);
            }
            0x1E | 0x3E => {
                let _ = self.cmp(self.x, value);
            }
            0x5E | 0x7E => {
                let _ = self.cmp(self.y, value);
            }
            0x8D => self.y = self.ld(self.y, value),
            0xAD => {
                let _ = self.cmp(self.y, value);
            }
            0xC8 => {
                let _ = self.cmp(self.x, value);
            }
            0xCD => self.x = self.ld(self.x, value),
            0xE9 => self.x = self.ld(self.x, value),
            0xEB => self.y = self.ld(self.y, value),
            0xEC => self.y = self.ld(self.y, value),
            0xF8 => self.x = self.ld(self.x, value),
            0xF9 => self.x = self.ld(self.x, value),
            0xFB => self.y = self.ld(self.y, value),
            _ => {}
        }
    }

    fn register(&self, register: Register) -> u8 {
        match register {
            Register::A => self.a,
            Register::X => self.x,
            Register::Y => self.y,
            Register::SP => self.sp,
            Register::P => self.psw(),
        }
    }

    fn set_register(&mut self, register: Register, value: u8) {
        match register {
            Register::A => self.a = value,
            Register::X => self.x = value,
            Register::Y => self.y = value,
            Register::SP => self.sp = value,
            Register::P => self.set_psw(value),
        }
    }

    // Remaining instruction helpers are defined below.
    fn direct_modify(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        let result = self.apply_modify(opcode, value);
        self.store(bus, address, result);
    }

    fn direct_indexed_modify(&mut self, bus: &mut impl SpcBus, opcode: u8, register: Register) {
        let address = self.fetch(bus);
        self.idle(bus);
        let target = address.wrapping_add(self.register(register));
        let value = self.load(bus, target);
        let result = self.apply_modify(opcode, value);
        self.store(bus, target, result);
    }

    fn absolute_modify(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let address = self.fetch16(bus);
        let value = self.read(bus, address);
        let result = self.apply_modify(opcode, value);
        self.write(bus, address, result);
    }

    fn implied_modify(&mut self, bus: &mut impl SpcBus, opcode: u8, register: Register) {
        self.dummy_read(bus);
        let value = self.register(register);
        let result = self.apply_modify(opcode, value);
        self.set_register(register, result);
    }

    fn apply_modify(&mut self, opcode: u8, value: u8) -> u8 {
        match opcode {
            0x0B | 0x0C | 0x1B | 0x1C => self.asl(value),
            0x2B | 0x2C | 0x3B | 0x3C => self.rol(value),
            0x4B | 0x4C | 0x5B | 0x5C => self.lsr(value),
            0x6B | 0x6C | 0x7B | 0x7C => self.ror(value),
            0x8B | 0x8C | 0x9B | 0x9C | 0x1D | 0xDC => self.dec(value),
            0xAB | 0xAC | 0xBB | 0xBC | 0x3D | 0xFC => self.inc(value),
            _ => value,
        }
    }

    fn direct_direct_modify(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let source = self.fetch(bus);
        let right = self.load(bus, source);
        let target = self.fetch(bus);
        let left = self.load(bus, target);
        let result = self.apply_binary(opcode, left, right);
        self.store(bus, target, result);
    }

    fn direct_immediate_modify(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let immediate = self.fetch(bus);
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        let result = self.apply_binary(opcode, value, immediate);
        self.store(bus, address, result);
    }

    fn direct_direct_compare(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let source = self.fetch(bus);
        let right = self.load(bus, source);
        let target = self.fetch(bus);
        let left = self.load(bus, target);
        let _ = self.apply_binary(opcode, left, right);
        self.idle(bus);
    }

    fn direct_immediate_compare(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let immediate = self.fetch(bus);
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        let _ = self.apply_binary(opcode, value, immediate);
        self.idle(bus);
    }

    fn apply_binary(&mut self, opcode: u8, left: u8, right: u8) -> u8 {
        match opcode {
            0x09 | 0x18 | 0x19 => self.or(left, right),
            0x27 | 0x29 | 0x38 | 0x39 => self.and(left, right),
            0x47 | 0x49 | 0x58 | 0x59 => self.eor(left, right),
            0x67 | 0x69 | 0x78 | 0x79 => self.cmp(left, right),
            0x87 | 0x89 | 0x98 | 0x99 => self.adc(left, right),
            0xA7 | 0xA9 | 0xB8 | 0xB9 => self.sbc(left, right),
            _ => left,
        }
    }

    fn direct_write(&mut self, bus: &mut impl SpcBus, register: Register) {
        let address = self.fetch(bus);
        self.load(bus, address);
        self.store(bus, address, self.register(register));
    }

    fn direct_immediate_write(&mut self, bus: &mut impl SpcBus) {
        let immediate = self.fetch(bus);
        let address = self.fetch(bus);
        self.load(bus, address);
        self.store(bus, address, immediate);
    }

    fn direct_direct_write(&mut self, bus: &mut impl SpcBus) {
        let source = self.fetch(bus);
        let value = self.load(bus, source);
        let target = self.fetch(bus);
        self.store(bus, target, value);
    }

    fn direct_indexed_write(&mut self, bus: &mut impl SpcBus, source: Register, index: Register) {
        let address = self.fetch(bus);
        self.idle(bus);
        let target = address.wrapping_add(self.register(index));
        self.load(bus, target);
        self.store(bus, target, self.register(source));
    }

    fn absolute_write(&mut self, bus: &mut impl SpcBus, register: Register) {
        let address = self.fetch16(bus);
        self.read(bus, address);
        self.write(bus, address, self.register(register));
    }

    fn absolute_indexed_write(&mut self, bus: &mut impl SpcBus, index: Register) {
        let address = self.fetch16(bus);
        self.idle(bus);
        let target = address.wrapping_add(u16::from(self.register(index)));
        self.read(bus, target);
        self.write(bus, target, self.a);
    }

    fn indirect_x_write(&mut self, bus: &mut impl SpcBus, register: Register) {
        self.idle(bus);
        self.load(bus, self.x);
        self.store(bus, self.x, self.register(register));
    }

    fn indexed_indirect_write(&mut self, bus: &mut impl SpcBus, source: Register, index: Register) {
        let indirect = self.fetch(bus);
        self.idle(bus);
        let pointer = self.load16(bus, indirect.wrapping_add(self.register(index)));
        self.read(bus, pointer);
        self.write(bus, pointer, self.register(source));
    }

    fn indirect_indexed_write(&mut self, bus: &mut impl SpcBus, source: Register, index: Register) {
        let indirect = self.fetch(bus);
        let pointer = self.load16(bus, indirect);
        self.idle(bus);
        let target = pointer.wrapping_add(u16::from(self.register(index)));
        self.read(bus, target);
        self.write(bus, target, self.register(source));
    }

    fn indirect_x_write_indirect_y(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        self.idle(bus);
        let right = self.load(bus, self.y);
        let left = self.load(bus, self.x);
        let result = self.apply_binary(opcode, left, right);
        self.store(bus, self.x, result);
    }

    fn indirect_x_compare_indirect_y(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        self.idle(bus);
        let right = self.load(bus, self.y);
        let left = self.load(bus, self.x);
        let _ = self.apply_binary(opcode, left, right);
        self.idle(bus);
    }

    fn indirect_x_increment_read(&mut self, bus: &mut impl SpcBus, register: Register) {
        self.idle(bus);
        let value = self.load(bus, self.x);
        self.x = self.x.wrapping_add(1);
        self.idle(bus);
        let value = self.ld(0, value);
        self.set_register(register, value);
    }

    fn indirect_x_increment_write(&mut self, bus: &mut impl SpcBus, register: Register) {
        self.idle(bus);
        self.idle(bus);
        self.store(bus, self.x, self.register(register));
        self.x = self.x.wrapping_add(1);
    }

    fn direct_write_word(&mut self, bus: &mut impl SpcBus) {
        let address = self.fetch(bus);
        self.load(bus, address);
        self.store(bus, address, self.a);
        self.store(bus, address.wrapping_add(1), self.y);
    }

    fn direct_read_word(&mut self, bus: &mut impl SpcBus, opcode: u8) {
        let address = self.fetch(bus);
        let low = self.load(bus, address);
        self.idle(bus);
        let high = self.load(bus, address.wrapping_add(1));
        let value = u16::from_le_bytes([low, high]);
        let result = match opcode {
            0x7A => self.adw(self.ya(), value),
            0x9A => self.sbw(self.ya(), value),
            0xBA => self.ldw(self.ya(), value),
            _ => self.ya(),
        };
        self.set_ya(result);
    }

    fn direct_compare_word(&mut self, bus: &mut impl SpcBus) {
        let address = self.fetch(bus);
        let low = self.load(bus, address);
        let high = self.load(bus, address.wrapping_add(1));
        let value = u16::from_le_bytes([low, high]);
        let _ = self.cpaw(self.ya(), value);
    }

    fn cpaw(&mut self, left: u16, right: u16) -> u16 {
        self.cpw(left, right)
    }

    fn direct_modify_word(&mut self, bus: &mut impl SpcBus, adjust: i16) {
        // ares `instructionDirectModifyWord`: load/store the low byte, then
        // load/store the high byte. Word flags come from the full result.
        let address = self.fetch(bus);
        let mut data = u16::from(self.load(bus, address)).wrapping_add(adjust as u16);
        self.store(bus, address, data as u8);
        data = data.wrapping_add(u16::from(self.load(bus, address.wrapping_add(1))) << 8);
        self.store(bus, address.wrapping_add(1), (data >> 8) as u8);
        self.flags.zero = data == 0;
        self.flags.negative = data & 0x8000 != 0;
    }

    fn jump_absolute(&mut self, bus: &mut impl SpcBus) {
        self.pc = self.fetch16(bus);
    }

    fn jump_indirect_x(&mut self, bus: &mut impl SpcBus) {
        let address = self.fetch16(bus);
        self.idle(bus);
        let target = self.read(bus, address.wrapping_add(u16::from(self.x)));
        let high = self.read(bus, address.wrapping_add(u16::from(self.x)).wrapping_add(1));
        self.pc = u16::from_le_bytes([target, high]);
    }

    fn call_absolute(&mut self, bus: &mut impl SpcBus) {
        let target = self.fetch16(bus);
        self.idle(bus);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.idle(bus);
        self.idle(bus);
        self.pc = target;
    }

    fn call_page(&mut self, bus: &mut impl SpcBus) {
        let target = self.fetch(bus);
        self.idle(bus);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.idle(bus);
        self.pc = 0xFF00 | u16::from(target);
    }

    fn call_table(&mut self, bus: &mut impl SpcBus, vector: u8) {
        self.idle(bus);
        self.idle(bus);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.idle(bus);
        let address = 0xFFDE - (u16::from(vector) << 1);
        let low = self.read(bus, address);
        let high = self.read(bus, address.wrapping_add(1));
        self.pc = u16::from_le_bytes([low, high]);
    }

    fn return_subroutine(&mut self, bus: &mut impl SpcBus) {
        // ares `instructionReturnSubroutine`: read(PC) then idle before the pulls.
        self.dummy_read(bus);
        self.idle(bus);
        let low = self.pull(bus);
        let high = self.pull(bus);
        self.pc = u16::from_le_bytes([low, high]);
    }

    fn return_interrupt(&mut self, bus: &mut impl SpcBus) {
        // ares `instructionReturnInterrupt`: read(PC) then idle before the pulls.
        self.dummy_read(bus);
        self.idle(bus);
        let psw = self.pull(bus);
        self.set_psw(psw);
        let low = self.pull(bus);
        let high = self.pull(bus);
        self.pc = u16::from_le_bytes([low, high]);
    }

    fn break_interrupt(&mut self, bus: &mut impl SpcBus) {
        self.dummy_read(bus);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.push(bus, self.psw());
        self.idle(bus);
        let low = self.read(bus, 0xFFDE);
        let high = self.read(bus, 0xFFDF);
        self.pc = u16::from_le_bytes([low, high]);
        self.flags.irq_disable = false;
        self.flags.break_flag = true;
    }

    fn push_reg(&mut self, bus: &mut impl SpcBus, register: Register) {
        self.dummy_read(bus);
        let value = self.register(register);
        self.push(bus, value);
        self.idle(bus);
    }

    fn pull_register(&mut self, bus: &mut impl SpcBus, register: Register) {
        self.dummy_read(bus);
        self.idle(bus);
        let value = self.pull(bus);
        match register {
            Register::P => self.set_psw(value),
            _ => self.set_register(register, value),
        }
    }

    fn pull_p(&mut self, bus: &mut impl SpcBus) {
        self.dummy_read(bus);
        self.idle(bus);
        let psw = self.pull(bus);
        self.set_psw(psw);
    }

    fn flag_set(&mut self, bus: &mut impl SpcBus, flag: Flag, value: bool) {
        self.dummy_read(bus);
        if flag == Flag::InterruptDisable {
            self.idle(bus);
        }
        match flag {
            Flag::Carry => self.flags.carry = value,
            Flag::Page => self.flags.page = value,
            Flag::InterruptDisable => self.flags.irq_disable = value,
        }
    }

    fn transfer(&mut self, bus: &mut impl SpcBus, from: Register, to: Register) {
        self.dummy_read(bus);
        let value = self.register(from);
        if to == Register::SP {
            self.sp = value;
            return;
        }
        self.set_register(to, value);
        self.set_zn(value);
    }

    fn complement_carry(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.idle(bus);
        self.flags.carry = !self.flags.carry;
    }

    fn overflow_clear(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.flags.overflow = false;
        self.flags.half_carry = false;
    }

    fn decimal_adjust_add(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.idle(bus);
        if self.flags.carry || self.a > 0x99 {
            self.a = self.a.wrapping_add(0x60);
            self.flags.carry = true;
        }
        if self.flags.half_carry || (self.a & 0x0F) > 0x09 {
            self.a = self.a.wrapping_add(0x06);
        }
        self.set_zn(self.a);
    }

    fn decimal_adjust_sub(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.idle(bus);
        if !self.flags.carry || self.a > 0x99 {
            self.a = self.a.wrapping_sub(0x60);
            self.flags.carry = false;
        }
        if !self.flags.half_carry || (self.a & 0x0F) > 0x09 {
            self.a = self.a.wrapping_sub(0x06);
        }
        self.set_zn(self.a);
    }

    fn exchange_nibble(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.idle(bus);
        self.idle(bus);
        self.idle(bus);
        self.a = self.a.rotate_left(4);
        self.set_zn(self.a);
    }

    fn divide(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        for _ in 0..10 {
            self.idle(bus);
        }
        let ya = self.ya();
        self.flags.half_carry = (self.y & 0x0F) >= (self.x & 0x0F);
        self.flags.overflow = self.y >= self.x;
        if u16::from(self.y) < (u16::from(self.x) << 1) {
            if self.x != 0 {
                self.a = (ya / u16::from(self.x)) as u8;
                self.y = (ya % u16::from(self.x)) as u8;
            }
        } else if self.x != 0xFF {
            let denominator = 256 - u16::from(self.x);
            let numerator = ya as i32 - (i32::from(self.x) << 9);
            self.a = 255 - (numerator / denominator as i32) as u8;
            self.y = self.x.wrapping_add((numerator % denominator as i32) as u8);
        }
        self.flags.zero = self.a == 0;
        self.flags.negative = self.a & 0x80 != 0;
    }

    fn multiply(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        for _ in 0..7 {
            self.idle(bus);
        }
        let result = u16::from(self.y) * u16::from(self.a);
        self.a = result as u8;
        self.y = (result >> 8) as u8;
        self.flags.zero = self.y == 0;
        self.flags.negative = self.y & 0x80 != 0;
    }

    fn wait_instruction(&mut self, bus: &mut impl SpcBus) {
        self.wait = true;
        // ares `instructionSleep` loops `read(PC); idle()` until the thread
        // synchronizes; its measured cost is 7 accesses including the fetch.
        for _ in 0..3 {
            self.dummy_read(bus);
            self.idle(bus);
        }
    }

    fn stop_instruction(&mut self, bus: &mut impl SpcBus) {
        self.stop = true;
        // ares `instructionStop` loops `read(PC); idle()` until the thread
        // synchronizes; its measured cost is 7 accesses including the fetch.
        for _ in 0..3 {
            self.dummy_read(bus);
            self.idle(bus);
        }
    }

    fn branch_bit(&mut self, bus: &mut impl SpcBus, bit: u8, match_value: bool) {
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        self.idle(bus);
        let displacement = self.fetch(bus) as i8;
        if ((value >> bit) & 1 != 0) != match_value {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }

    fn absolute_bit_set(&mut self, bus: &mut impl SpcBus, bit: u8, value: bool) {
        let address = self.fetch(bus);
        let mut data = self.load(bus, address);
        if value {
            data |= 1 << bit;
        } else {
            data &= !(1 << bit);
        }
        self.store(bus, address, data);
    }

    fn absolute_bit_modify(&mut self, bus: &mut impl SpcBus, mode: u8) {
        let address = self.fetch16(bus);
        let bit = (address >> 13) as u8;
        let address = address & 0x1FFF;
        let mut data = self.read(bus, address);
        match mode {
            0 => {
                self.idle(bus);
                self.flags.carry |= (data >> bit) & 1 != 0;
            }
            1 => {
                self.idle(bus);
                self.flags.carry |= (data >> bit) & 1 == 0;
            }
            2 => self.flags.carry &= (data >> bit) & 1 != 0,
            3 => self.flags.carry &= (data >> bit) & 1 == 0,
            4 => {
                self.idle(bus);
                self.flags.carry ^= (data >> bit) & 1 != 0;
            }
            5 => self.flags.carry = (data >> bit) & 1 != 0,
            6 => {
                self.idle(bus);
                if self.flags.carry {
                    data |= 1 << bit;
                } else {
                    data &= !(1 << bit);
                }
                self.write(bus, address, data);
            }
            7 => {
                data ^= 1 << bit;
                self.write(bus, address, data);
            }
            _ => {}
        }
    }

    fn test_set_bits_absolute(&mut self, bus: &mut impl SpcBus, set: bool) {
        let address = self.fetch16(bus);
        let data = self.read(bus, address);
        self.flags.zero = self.a.wrapping_sub(data) == 0;
        self.flags.negative = self.a.wrapping_sub(data) & 0x80 != 0;
        self.read(bus, address);
        let result = if set { data | self.a } else { data & !self.a };
        self.write(bus, address, result);
    }

    fn branch_not_direct(&mut self, bus: &mut impl SpcBus) {
        let address = self.fetch(bus);
        let value = self.load(bus, address);
        self.idle(bus);
        let displacement = self.fetch(bus) as i8;
        if self.a == value {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }

    fn branch_not_direct_decrement(&mut self, bus: &mut impl SpcBus) {
        let address = self.fetch(bus);
        let value = self.load(bus, address).wrapping_sub(1);
        self.store(bus, address, value);
        let displacement = self.fetch(bus) as i8;
        if value == 0 {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }

    fn branch_not_direct_indexed(&mut self, bus: &mut impl SpcBus, register: Register) {
        let address = self.fetch(bus);
        self.idle(bus);
        let value = self.load(bus, address.wrapping_add(self.register(register)));
        self.idle(bus);
        let displacement = self.fetch(bus) as i8;
        if self.a == value {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }

    fn branch_not_y_decrement(&mut self, bus: &mut impl SpcBus) {
        self.idle(bus);
        self.idle(bus);
        let displacement = self.fetch(bus) as i8;
        self.y = self.y.wrapping_sub(1);
        if self.y == 0 {
            return;
        }
        self.idle(bus);
        self.idle(bus);
        self.pc = self.pc.wrapping_add(displacement as u16);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Register {
    A,
    X,
    Y,
    SP,
    P,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flag {
    Carry,
    Page,
    InterruptDisable,
}

impl fmt::Display for Spc700 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PC={:04X} A={:02X} X={:02X} Y={:02X} SP={:02X} PSW={:02X}",
            self.pc,
            self.a,
            self.x,
            self.y,
            self.sp,
            self.psw()
        )
    }
}
