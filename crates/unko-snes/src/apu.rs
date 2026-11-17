//! S-SMP APU wrapper: 64 KiB APU RAM, IPL ROM, CPU ports and the S-DSP stub.
//!
//! The SPC700 CPU is implemented in [`crate::spc700`]. This module owns the
//! APU address space and the four bidirectional S-CPU ports. Audio synthesis is
//! not implemented yet; the DSP register file is present so games can upload
//! their sound driver and complete the boot handshake.

use std::collections::VecDeque;

use crate::dsp::{Dsp, Sample};
use crate::spc700::{Spc700, SpcBus};

/// Size of APU RAM.
pub const APU_RAM_SIZE: usize = 0x1_0000;

/// Size of the SPC700 IPL ROM.
pub const IPL_SIZE: usize = 64;

/// One S-SMP timer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timer {
    stage0: u16,
    stage1: bool,
    stage2: u8,
    stage3: u8,
    line: bool,
    enable: bool,
    target: u8,
}

impl Timer {
    /// Re-evaluate the timer line level without advancing stage 0 (ares
    /// `synchronizeStage1`), used when `$F0` changes the enable bits.
    fn synchronize(&mut self, timers_enable: bool, timers_disable: bool) {
        let level = self.stage1 && timers_enable && !timers_disable;
        if self.line && !level && self.enable {
            self.stage2 = self.stage2.wrapping_add(1);
            if self.stage2 == self.target {
                self.stage2 = 0;
                self.stage3 = (self.stage3 + 1) & 0x0F;
            }
        }
        self.line = level;
    }

    fn step(&mut self, clocks: u32, frequency: u16, timers_enable: bool, timers_disable: bool) {
        self.stage0 = self.stage0.wrapping_add(clocks as u16);
        while self.stage0 >= frequency {
            self.stage0 -= frequency;
            self.stage1 = !self.stage1;
            let level = self.stage1 && timers_enable && !timers_disable;
            if self.line && !level && self.enable {
                self.stage2 = self.stage2.wrapping_add(1);
                if self.stage2 == self.target {
                    self.stage2 = 0;
                    self.stage3 = (self.stage3 + 1) & 0x0F;
                }
            }
            self.line = level;
        }
    }
}

/// One bus access of the SPC700 instruction currently being drained.
///
/// ares runs the S-SMP as a cooperative thread: every access steps the timers
/// and the DSP by its wait-state count, and an S-SMP-to-S-CPU port write becomes
/// visible to the S-CPU at the exact clock the access completes. Recording the
/// access timeline lets us commit those port latches at the same instants while
/// the S-CPU is in the middle of its own instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlanStep {
    /// S-SMP clocks (2.048 MHz) this access consumes.
    clocks: u8,
    /// Latched `$F4-$F7` write, committed when the access finishes.
    port_write: Option<(u8, u8)>,
}

/// The S-SMP + S-DSP APU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apu {
    /// SPC700 CPU state.
    pub spc: Spc700,
    ram: Box<[u8; APU_RAM_SIZE]>,
    ipl: [u8; IPL_SIZE],
    ipl_loaded: bool,
    iplrom_enable: bool,
    ram_writable: bool,
    ram_disable: bool,
    timers_enable: bool,
    timers_disable: bool,
    internal_wait: u8,
    external_wait: u8,
    timers: [Timer; 3],
    dsp_address: u8,
    dsp: Dsp,
    /// Ports written by the S-CPU and read by the SPC700.
    apu_ports: [u8; 4],
    /// Ports written by the SPC700 and read by the S-CPU.
    cpu_ports: [u8; 4],
    aux4: u8,
    aux5: u8,
    timer_targets: [u8; 3],
    /// Number of SPC700 instructions executed (debug).
    pub instructions: u64,
    /// Number of S-SMP clock units (two clocks each) retired (debug).
    pub units: u64,
    /// Micro-op timeline of the instruction being drained.
    plan: Vec<PlanStep>,
    plan_pos: usize,
    plan_sub: u32,
    samples: VecDeque<Sample>,
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl Apu {
    /// Create an APU without an IPL ROM. A user-supplied `ipl.rom` is required
    /// for the real boot protocol.
    pub fn new() -> Self {
        Self {
            spc: Spc700::new(),
            ram: Box::new([0; APU_RAM_SIZE]),
            ipl: [0; IPL_SIZE],
            ipl_loaded: false,
            iplrom_enable: true,
            ram_writable: true,
            ram_disable: false,
            timers_enable: true,
            timers_disable: false,
            internal_wait: 0,
            external_wait: 0,
            timers: std::array::from_fn(|_| Timer::default()),
            dsp_address: 0,
            dsp: Dsp::new(),
            apu_ports: [0; 4],
            cpu_ports: [0; 4],
            aux4: 0,
            aux5: 0,
            timer_targets: [0; 3],
            instructions: 0,
            units: 0,
            plan: Vec::new(),
            plan_pos: 0,
            plan_sub: 0,
            samples: VecDeque::new(),
        }
    }

    /// Create an APU with a 64-byte IPL ROM.
    pub fn with_ipl(ipl: [u8; IPL_SIZE]) -> Self {
        let mut apu = Self::new();
        apu.set_ipl(ipl);
        apu
    }

    /// Install a 64-byte IPL ROM.
    pub fn set_ipl(&mut self, ipl: [u8; IPL_SIZE]) {
        self.ipl = ipl;
        self.ipl_loaded = true;
        self.reset();
    }

    /// Whether an IPL ROM is installed.
    pub const fn has_ipl(&self) -> bool {
        self.ipl_loaded
    }

    /// APU RAM.
    pub fn ram(&self) -> &[u8] {
        &self.ram[..]
    }

    /// Mutable APU RAM.
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram[..]
    }

    /// S-DSP register file.
    pub fn dsp(&self) -> &Dsp {
        &self.dsp
    }

    /// Mutable S-DSP register file.
    pub fn dsp_mut(&mut self) -> &mut Dsp {
        &mut self.dsp
    }

    /// Reset the APU and jump to the IPL ROM reset vector.
    pub fn reset(&mut self) {
        self.spc.reset();
        self.iplrom_enable = true;
        self.ram_writable = true;
        self.ram_disable = false;
        self.timers_enable = true;
        self.timers_disable = false;
        self.internal_wait = 0;
        self.external_wait = 0;
        self.timers = std::array::from_fn(|_| Timer::default());
        self.dsp_address = 0;
        self.dsp.reset();
        self.plan.clear();
        self.plan_pos = 0;
        self.plan_sub = 0;
        self.samples.clear();
        self.apu_ports = [0; 4];
        self.cpu_ports = [0; 4];
        self.aux4 = 0;
        self.aux5 = 0;
        self.timer_targets = [0; 3];
        if self.ipl_loaded {
            self.spc.pc = u16::from_le_bytes([self.ipl[62], self.ipl[63]]);
        } else {
            self.spc.pc = 0x0200;
        }
        self.spc.sp = 0xEF;
    }

    /// S-CPU read from `$2140-$2143`.
    pub fn cpu_read(&self, port: usize) -> u8 {
        self.cpu_ports[port & 3]
    }

    /// S-CPU write to `$2140-$2143`.
    pub fn cpu_write(&mut self, port: usize, value: u8) {
        self.apu_ports[port & 3] = value;
    }

    /// Value currently visible to the SPC700 on `$F4-$F7`.
    pub fn apu_port(&self, port: usize) -> u8 {
        self.apu_ports[port & 3]
    }

    /// Set a value visible to the S-CPU on `$2140-$2143` (test/debug helper).
    pub fn set_cpu_port(&mut self, port: usize, value: u8) {
        self.cpu_ports[port & 3] = value;
    }

    /// Execute one SPC700 instruction.
    pub fn step(&mut self) -> u32 {
        if self.plan_pos >= self.plan.len() {
            self.record_instruction();
        }
        let mut total = 0;
        while self.plan_pos < self.plan.len() {
            total += self.drain_unit();
        }
        total
    }

    /// Advance the S-SMP by one base-clock unit (two 2.048 MHz clocks).
    ///
    /// The system calls this from inside an S-CPU access, so an S-SMP port write
    /// commits at the exact master clock of the S-SMP access that produced it.
    pub fn tick(&mut self) -> u32 {
        self.drain_unit()
    }

    /// Execute the next instruction into a fresh micro-op timeline.
    fn record_instruction(&mut self) {
        self.plan.clear();
        self.plan_pos = 0;
        self.plan_sub = 0;
        let page_flag = self.spc.flags.page;
        // The S-SMP's logical time is exactly its retired access units.
        let mut plan = std::mem::take(&mut self.plan);
        {
            let mut bus = ApuBus {
                ram: &mut self.ram[..],
                ipl: &self.ipl,
                ipl_loaded: self.ipl_loaded,
                iplrom_enable: &mut self.iplrom_enable,
                ram_writable: &mut self.ram_writable,
                ram_disable: &mut self.ram_disable,
                timers_enable: &mut self.timers_enable,
                timers_disable: &mut self.timers_disable,
                internal_wait: &mut self.internal_wait,
                external_wait: &mut self.external_wait,
                timers: &mut self.timers,
                dsp_address: &mut self.dsp_address,
                dsp: &mut self.dsp,
                apu_ports: &mut self.apu_ports,
                cpu_ports: &mut self.cpu_ports,
                aux4: &mut self.aux4,
                aux5: &mut self.aux5,
                timer_targets: &mut self.timer_targets,
                page_flag,
            };
            let mut recorder = RecordBus {
                inner: &mut bus,
                plan: &mut plan,
                units: 0,
            };
            self.spc.step(&mut recorder);
        }
        if plan.is_empty() {
            plan.push(PlanStep {
                clocks: 2,
                port_write: None,
            });
        }
        self.plan = plan;
        self.instructions += 1;
    }

    /// Drain one base-clock unit (two clocks) from the recorded timeline.
    fn drain_unit(&mut self) -> u32 {
        if self.plan_pos >= self.plan.len() {
            self.record_instruction();
        }
        self.units += 1;
        let step = self.plan[self.plan_pos];
        self.plan_sub += 2;
        if self.plan_sub >= u32::from(step.clocks) {
            if let Some((port, value)) = step.port_write {
                // S-SMP -> S-CPU latch ($2140-$2143).
                self.cpu_ports[usize::from(port)] = value;
            }
            self.plan_pos += 1;
            self.plan_sub = 0;
        }
        // ares synchronizes the DSP thread on every S-SMP access and its tick
        // is exactly one access unit, so the DSP advances one micro-step here
        // and finishes a sample every 32 accesses. Advancing the whole sample
        // at once would hide the echo buffer's partial state from an S-SMP
        // read that lands mid-sample.
        if let Some(sample) = self.dsp.tick(&mut self.ram[..]) {
            self.samples.push_back(sample);
            if self.samples.len() > 16_384 {
                self.samples.pop_front();
            }
        }
        2
    }

    /// Number of queued DSP samples.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Drain queued DSP samples.
    pub fn drain_samples(&mut self) -> impl Iterator<Item = Sample> + '_ {
        self.samples.drain(..)
    }

    /// Timer internals for diagnostics: `(stage0, stage1, stage2, stage3,
    /// target, enable)`.
    pub fn timer_state(&self, index: usize) -> (u16, bool, u8, u8, u8, bool) {
        let timer = &self.timers[index];
        (
            timer.stage0,
            timer.stage1,
            timer.stage2,
            timer.stage3,
            timer.target,
            timer.enable,
        )
    }

    /// Global timer enable/disable latches for diagnostics.
    pub const fn timer_latches(&self) -> (bool, bool) {
        (self.timers_enable, self.timers_disable)
    }

    /// Queued DSP samples without draining them.
    pub fn samples(&self) -> &VecDeque<Sample> {
        &self.samples
    }

    /// Run the S-SMP until at least `cycles` 2.048 MHz clocks have elapsed.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let mut total = 0;
        while total < cycles {
            total += u64::from(self.drain_unit());
        }
        total
    }
}

struct ApuBus<'a> {
    ram: &'a mut [u8],
    ipl: &'a [u8; IPL_SIZE],
    ipl_loaded: bool,
    iplrom_enable: &'a mut bool,
    ram_writable: &'a mut bool,
    ram_disable: &'a mut bool,
    timers_enable: &'a mut bool,
    timers_disable: &'a mut bool,
    internal_wait: &'a mut u8,
    external_wait: &'a mut u8,
    timers: &'a mut [Timer; 3],
    dsp_address: &'a mut u8,
    dsp: &'a mut Dsp,
    apu_ports: &'a mut [u8; 4],
    cpu_ports: &'a mut [u8; 4],
    aux4: &'a mut u8,
    aux5: &'a mut u8,
    timer_targets: &'a mut [u8; 3],
    page_flag: bool,
}

impl ApuBus<'_> {
    /// Write the underlying APU RAM (ares `writeRAM`).
    fn write_ram(&mut self, address: u16, value: u8) {
        if *self.ram_writable && !*self.ram_disable {
            self.ram[usize::from(address)] = value;
        }
    }

    /// Wait-state selector for an access: RAM uses the external dividers;
    /// I/O, the IPL ROM and idle cycles use the internal dividers
    /// (ares `SMP::wait`).
    fn wait_selector(&self, address: Option<u16>) -> u8 {
        match address {
            None => *self.internal_wait,
            Some(address)
                if address & 0xFFF0 == 0x00F0 || (address >= 0xFFC0 && *self.iplrom_enable) =>
            {
                *self.internal_wait
            }
            _ => *self.external_wait,
        }
        .min(3)
    }
}

impl SpcBus for ApuBus<'_> {
    fn advance(&mut self, clocks: u32) {
        for (index, timer) in self.timers.iter_mut().enumerate() {
            let frequency = if index == 2 { 16 } else { 128 };
            timer.step(clocks, frequency, *self.timers_enable, *self.timers_disable);
        }
    }

    fn wait_cycles(&mut self, address: Option<u16>) -> u32 {
        // Time is accounted in access units (one unit is two S-SMP clocks).
        // ares `cycleWaitStates` is 2/4/10/20 clocks: the 8/16 settings cost
        // 10/20 on silicon, the documented hardware glitch.
        const CYCLE_WAIT: [u32; 4] = [1, 2, 5, 10];
        CYCLE_WAIT[usize::from(self.wait_selector(address))]
    }

    fn timer_cycles(&mut self, address: Option<u16>) -> u32 {
        // ares `timerWaitStates`: the timers are not affected by the glitch.
        const TIMER_WAIT: [u32; 4] = [2, 4, 8, 16];
        TIMER_WAIT[usize::from(self.wait_selector(address))]
    }

    fn split_port_read(&mut self, address: u16) -> bool {
        address & 0xFFFC == 0x00F4
    }

    fn read(&mut self, address: u16) -> u8 {
        match address {
            0xF0 | 0xF1 | 0xFA | 0xFB | 0xFC => 0,
            0xF2 => *self.dsp_address,
            0xF3 => self.dsp.read(*self.dsp_address),
            0xF4..=0xF7 => self.apu_ports[usize::from(address - 0xF4)],
            0xF8 => *self.aux4,
            0xF9 => *self.aux5,
            0xFD..=0xFF => {
                let timer = &mut self.timers[usize::from(address - 0xFD)];
                let value = timer.stage3;
                timer.stage3 = 0;
                value
            }
            _ => {
                if self.ipl_loaded && *self.iplrom_enable && address >= 0xFFC0 {
                    self.ipl[usize::from(address & 0x3F)]
                } else if *self.ram_disable {
                    0x5A
                } else {
                    self.ram[usize::from(address)]
                }
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.write_ram(address, value);
        match address {
            0xF0 => {
                // ares: writes are only valid when the SPC700 P flag is clear.
                if self.page_flag {
                    return;
                }
                *self.timers_disable = value & 0x01 != 0;
                *self.ram_writable = value & 0x02 != 0;
                *self.ram_disable = value & 0x04 != 0;
                *self.timers_enable = value & 0x08 != 0;
                // ares: external wait = bits 4-5, internal wait = bits 6-7.
                *self.external_wait = (value >> 4) & 0x03;
                *self.internal_wait = (value >> 6) & 0x03;
                for timer in self.timers.iter_mut() {
                    timer.synchronize(*self.timers_enable, *self.timers_disable);
                }
            }
            0xF1 => {
                for (index, timer) in self.timers.iter_mut().enumerate() {
                    let enable = value & (1 << index) != 0;
                    let rising = enable && !timer.enable;
                    timer.enable = enable;
                    // ares resets timers 0/1 on a rising enable edge, but
                    // timer 2 on every write that is not a rising edge.
                    if rising != (index == 2) {
                        timer.stage2 = 0;
                        timer.stage3 = 0;
                    }
                }
                // ares: `$F1` bit 4 clears the S-CPU`s `$2140/$2141` latches
                // and bit 5 clears `$2142/$2143` (the sound driver's port
                // reset). The SMP reads these at `$F4-$F7`.
                if value & 0x10 != 0 {
                    self.apu_ports[0] = 0;
                    self.apu_ports[1] = 0;
                }
                if value & 0x20 != 0 {
                    self.apu_ports[2] = 0;
                    self.apu_ports[3] = 0;
                }
                *self.iplrom_enable = value & 0x80 != 0;
                if value & 0x10 != 0 {
                    self.apu_ports[0] = 0;
                    self.apu_ports[1] = 0;
                }
                if value & 0x20 != 0 {
                    self.apu_ports[2] = 0;
                    self.apu_ports[3] = 0;
                }
            }
            0xF2 => *self.dsp_address = value,
            0xF3 => self.dsp.write(*self.dsp_address, value),
            0xF4..=0xF7 => self.cpu_ports[usize::from(address - 0xF4)] = value,
            0xF8 => *self.aux4 = value,
            0xF9 => *self.aux5 = value,
            0xFA..=0xFC => {
                self.timer_targets[usize::from(address - 0xFA)] = value;
                self.timers[usize::from(address - 0xFA)].target = value;
            }
            _ => {}
        }
    }
}

/// Recording bus: runs one SPC700 instruction, applies every side effect
/// immediately except the S-SMP-to-S-CPU port latch, and records the access
/// timeline so [`Apu`] can drain it clock by clock.
struct RecordBus<'a, 'b> {
    inner: &'a mut ApuBus<'b>,
    plan: &'a mut Vec<PlanStep>,
    /// Access units recorded so far in this instruction.
    units: u64,
}

impl SpcBus for RecordBus<'_, '_> {
    fn wait_cycles(&mut self, address: Option<u16>) -> u32 {
        self.inner.wait_cycles(address)
    }

    fn timer_cycles(&mut self, address: Option<u16>) -> u32 {
        self.inner.timer_cycles(address)
    }

    fn split_port_read(&mut self, address: u16) -> bool {
        self.inner.split_port_read(address)
    }

    fn advance(&mut self, clocks: u32) {
        self.inner.advance(clocks);
    }

    fn read(&mut self, address: u16) -> u8 {
        let clocks = self.inner.timer_cycles(Some(address)) as u8;
        let value = self.inner.read(address);
        self.plan.push(PlanStep {
            clocks,
            port_write: None,
        });
        self.units += u64::from(clocks) / 2;
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        let clocks = self.inner.timer_cycles(Some(address)) as u8;
        self.units += u64::from(clocks) / 2;
        if (0xF4..=0xF7).contains(&address) {
            // ares writes the RAM mirror immediately and latches the port; the
            // latch itself is only visible to the S-CPU at the access's end.
            self.inner.write_ram(address, value);
            self.plan.push(PlanStep {
                clocks,
                port_write: Some(((address - 0xF4) as u8, value)),
            });
        } else {
            self.inner.write(address, value);
            self.plan.push(PlanStep {
                clocks,
                port_write: None,
            });
        }
    }

    fn idle(&mut self) {
        let clocks = self.inner.timer_cycles(None) as u8;
        self.plan.push(PlanStep {
            clocks,
            port_write: None,
        });
        self.units += u64::from(clocks) / 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipl() -> [u8; IPL_SIZE] {
        let mut ipl = [0u8; IPL_SIZE];
        // reset vector -> $FFC0
        ipl[62] = 0xC0;
        ipl[63] = 0xFF;
        ipl
    }

    #[test]
    fn reset_uses_ipl_vector() {
        let apu = Apu::with_ipl(ipl());
        assert!(apu.has_ipl());
        assert_eq!(apu.spc.pc, 0xFFC0);
        assert_eq!(apu.spc.sp, 0xEF);
    }

    #[test]
    fn cpu_ports_are_bidirectional() {
        let mut apu = Apu::new();
        apu.cpu_write(0, 0x12);
        assert_eq!(apu.apu_ports[0], 0x12);
        apu.cpu_ports[1] = 0x34;
        assert_eq!(apu.cpu_read(1), 0x34);
    }

    #[test]
    fn smp_port_reads_see_only_the_cpu_latches() {
        // ares keeps two independent four-byte latches: `io.apu0..3` written by
        // the S-CPU and read by the S-SMP (`$F4-$F7`), and `io.cpu0..3` written
        // by the S-SMP and read by the S-CPU (`$2140-$2143`). A program that
        // writes `$F6` and reads it back therefore sees the *CPU's* value, not
        // its own; blargg `spc_smp` test 9 depends on exactly that.
        let mut apu = Apu::new();
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
            };
            // The S-CPU has not written yet.
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0);
            // The S-SMP writes its own latch; a read still sees the CPU side.
            SpcBus::write(&mut bus, 0x00F6, 0xFF);
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0);
            *bus.apu_ports = [0, 0, 0x5A, 0];
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0x5A);
        }
        // And the S-SMP write became visible to the S-CPU.
        assert_eq!(apu.cpu_read(2), 0xFF);
    }

    #[test]
    fn control_clears_the_port_latches() {
        // ares `SMP::writeIO` case 0xf1: bit 4 clears `io.apu0`/`io.apu1` and
        // bit 5 clears `io.apu2`/`io.apu3` (the latches the S-SMP reads at
        // `$F4-$F7`, i.e. whatever the S-CPU last wrote to `$2140-$2143`).
        let mut apu = Apu::new();
        apu.cpu_write(0, 0xAA);
        apu.cpu_write(1, 0xBB);
        apu.cpu_write(2, 0xCC);
        apu.cpu_write(3, 0xDD);
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
            };
            SpcBus::write(&mut bus, 0x00F1, 0x10);
        }
        assert_eq!(apu.apu_ports, [0x00, 0x00, 0xCC, 0xDD]);
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
            };
            SpcBus::write(&mut bus, 0x00F1, 0x20);
        }
        assert_eq!(apu.apu_ports, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn control_disables_ipl_rom() {
        let mut apu = Apu::with_ipl(ipl());
        apu.ram_mut()[0xFFC0] = 0xAA;
        let mut bus = ApuBus {
            ram: &mut apu.ram[..],
            ipl: &apu.ipl,
            ipl_loaded: apu.ipl_loaded,
            iplrom_enable: &mut apu.iplrom_enable,
            ram_writable: &mut apu.ram_writable,
            ram_disable: &mut apu.ram_disable,
            timers_enable: &mut apu.timers_enable,
            timers_disable: &mut apu.timers_disable,
            internal_wait: &mut apu.internal_wait,
            external_wait: &mut apu.external_wait,
            timers: &mut apu.timers,
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
            page_flag: false,
        };
        assert_eq!(bus.read(0xFFC0), 0x00);
        bus.write(0xF1, 0x00);
        assert!(!*bus.iplrom_enable);
        assert_eq!(bus.read(0xFFC0), 0xAA);
    }

    #[test]
    fn executes_spc700_from_ram() {
        let mut apu = Apu::new();
        apu.spc.pc = 0x0200;
        apu.ram_mut()[0x0200] = 0xE8; // LDA #$42
        apu.ram_mut()[0x0201] = 0x42;
        apu.ram_mut()[0x0202] = 0x00; // NOP
        let cycles = apu.step();
        assert!(cycles >= 2);
        assert_eq!(apu.spc.a, 0x42);
    }
}

#[cfg(test)]
mod dsp_integration_tests {
    use super::*;

    #[test]
    fn apu_runs_dsp_when_spc_executes() {
        let mut apu = Apu::new();
        // NOP sled at $0500 so the SPC keeps retiring cycles.
        for byte in &mut apu.ram_mut()[0x0500..0x0580] {
            *byte = 0x00;
        }
        apu.spc.pc = 0x0500;
        // Minimal looping BRR sample at $0300.
        apu.ram_mut()[0x0200] = 0x00;
        apu.ram_mut()[0x0201] = 0x03;
        apu.ram_mut()[0x0202] = 0x00;
        apu.ram_mut()[0x0203] = 0x03;
        apu.ram_mut()[0x0300] = 0x03;
        for (index, byte) in [0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]
            .into_iter()
            .enumerate()
        {
            apu.ram_mut()[0x0301 + index] = byte;
        }
        {
            let dsp = apu.dsp_mut();
            dsp.write(0x0C, 0x7F);
            dsp.write(0x1C, 0x7F);
            dsp.write(0x00, 0x7F);
            dsp.write(0x01, 0x7F);
            dsp.write(0x02, 0x00);
            dsp.write(0x03, 0x10);
            dsp.write(0x04, 0x00);
            dsp.write(0x07, 0x7F);
            dsp.write(0x5D, 0x02);
            dsp.write(0x6C, 0x00);
            dsp.write(0x4C, 0x01);
        }
        let mut steps = 0;
        while apu.sample_count() < 64 && steps < 100_000 {
            apu.step();
            steps += 1;
        }
        assert!(apu.sample_count() >= 64, "DSP did not produce samples");
        assert!(apu.drain_samples().any(|sample| sample != (0, 0)));
    }
}

#[cfg(test)]
mod wait_state_tests {
    use super::*;

    fn bus<'a>(apu: &'a mut Apu) -> ApuBus<'a> {
        ApuBus {
            ram: &mut apu.ram[..],
            ipl: &apu.ipl,
            ipl_loaded: apu.ipl_loaded,
            iplrom_enable: &mut apu.iplrom_enable,
            ram_writable: &mut apu.ram_writable,
            ram_disable: &mut apu.ram_disable,
            timers_enable: &mut apu.timers_enable,
            timers_disable: &mut apu.timers_disable,
            internal_wait: &mut apu.internal_wait,
            external_wait: &mut apu.external_wait,
            timers: &mut apu.timers,
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
            page_flag: false,
        }
    }

    #[test]
    fn control_register_scales_access_time() {
        let mut apu = Apu::new();
        apu.ram_mut()[0x0200] = 0xE8; // LDA #$42
        apu.ram_mut()[0x0201] = 0x42;
        apu.spc.pc = 0x0200;
        // Two RAM accesses at two clocks each.
        assert_eq!(apu.step(), 4);

        apu.spc.pc = 0x0200;
        {
            let mut bus = bus(&mut apu);
            // External wait state 1 (bits 4-5): four clocks per access.
            bus.write(0xF0, 0x10);
        }
        assert_eq!(apu.step(), 8);

        apu.spc.pc = 0x0200;
        {
            let mut bus = bus(&mut apu);
            // Internal wait state 2 (bits 6-7) applies to I/O and idle cycles;
            // RAM keeps using the external divider.
            bus.write(0xF0, 0x80);
            assert_eq!(bus.wait_cycles(Some(0x00F4)), 5);
            assert_eq!(bus.wait_cycles(None), 5);
            assert_eq!(bus.wait_cycles(Some(0x0200)), 1);
            // The timers are not affected by the 10/20-clock glitch: an
            // internal wait state of 2 costs 8 timer clocks, not 10.
            assert_eq!(bus.timer_cycles(Some(0x00F4)), 8);
            assert_eq!(bus.timer_cycles(None), 8);
        }
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn timer_reaches_output() {
        let mut apu = Apu::new();
        apu.timers[0].target = 1;
        apu.timers[0].enable = true;
        for _ in 0..2000 {
            apu.timers[0].step(2, 128, true, false);
        }
        assert!(apu.timers[0].stage3 > 0);
    }
}
