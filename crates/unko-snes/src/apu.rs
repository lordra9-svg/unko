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
    dsp_address: u8,
    dsp: Dsp,
    /// Ports written by the S-CPU and read by the SPC700.
    apu_ports: [u8; 4],
    /// Ports written by the SPC700 and read by the S-CPU.
    cpu_ports: [u8; 4],
    aux4: u8,
    aux5: u8,
    timer_targets: [u8; 3],
    dsp_credit: u64,
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
            dsp_address: 0,
            dsp: Dsp::new(),
            apu_ports: [0; 4],
            cpu_ports: [0; 4],
            aux4: 0,
            aux5: 0,
            timer_targets: [0; 3],
            dsp_credit: 0,
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
        self.dsp_address = 0;
        self.dsp.reset();
        self.dsp_credit = 0;
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
        let mut bus = ApuBus {
            ram: &mut self.ram[..],
            ipl: &self.ipl,
            ipl_loaded: self.ipl_loaded,
            iplrom_enable: &mut self.iplrom_enable,
            ram_writable: &mut self.ram_writable,
            ram_disable: &mut self.ram_disable,
            timers_enable: &mut self.timers_enable,
            dsp_address: &mut self.dsp_address,
            dsp: &mut self.dsp,
            apu_ports: &mut self.apu_ports,
            cpu_ports: &mut self.cpu_ports,
            aux4: &mut self.aux4,
            aux5: &mut self.aux5,
            timer_targets: &mut self.timer_targets,
        };
        let cycles = self.spc.step(&mut bus);
        self.dsp_credit += u64::from(cycles);
        while self.dsp_credit >= 32 {
            self.dsp_credit -= 32;
            let sample = self.dsp.run_sample(&mut self.ram[..]);
            self.samples.push_back(sample);
            if self.samples.len() > 16_384 {
                self.samples.pop_front();
            }
        }
        cycles
    }

    /// Number of queued DSP samples.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Drain queued DSP samples.
    pub fn drain_samples(&mut self) -> impl Iterator<Item = Sample> + '_ {
        self.samples.drain(..)
    }

    /// Run SPC700 instructions until at least `cycles` SPC cycles have been
    /// retired.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let mut total = 0;
        while total < cycles {
            total += u64::from(self.step());
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
    dsp_address: &'a mut u8,
    dsp: &'a mut Dsp,
    apu_ports: &'a mut [u8; 4],
    cpu_ports: &'a mut [u8; 4],
    aux4: &'a mut u8,
    aux5: &'a mut u8,
    timer_targets: &'a mut [u8; 3],
}

impl SpcBus for ApuBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        match address {
            0xF0 | 0xF1 | 0xFA | 0xFB | 0xFC | 0xFD | 0xFE | 0xFF => 0,
            0xF2 => *self.dsp_address,
            0xF3 => self.dsp.read(*self.dsp_address),
            0xF4..=0xF7 => self.apu_ports[usize::from(address - 0xF4)],
            0xF8 => *self.aux4,
            0xF9 => *self.aux5,
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
        if *self.ram_writable && !*self.ram_disable {
            self.ram[usize::from(address)] = value;
        }
        match address {
            0xF0 => {
                *self.timers_enable = value & 0x08 != 0;
                *self.ram_writable = value & 0x02 != 0;
                *self.ram_disable = value & 0x04 != 0;
            }
            0xF1 => {
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
            0xFA..=0xFC => self.timer_targets[usize::from(address - 0xFA)] = value,
            _ => {}
        }
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
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
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
