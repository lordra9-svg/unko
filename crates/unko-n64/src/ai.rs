//! N64 audio interface (AI): DRAM/len/control/status registers, double-buffered
//! DMA from RDRAM, a DMA-completion interrupt and linear resampling from the
//! DAC rate to the host output rate.

/// AI register block base.
pub const BASE: u32 = 0x0450_0000;

/// One queued DMA buffer; samples are read from RDRAM as playback advances.
#[derive(Debug, Clone, Copy)]
struct DmaBuffer {
    address: usize,
    remaining: usize,
}

/// One AI.
#[derive(Debug, Clone)]
pub struct Ai {
    /// `AI_DRAM_ADDR`.
    pub dram_addr: u32,
    /// `AI_LEN` of the active buffer (bytes).
    pub len: u32,
    /// `AI_CONTROL`; bit 0 enables DMA.
    pub control: u32,
    /// `AI_DACRATE`.
    pub dacrate: u32,
    /// `AI_BITRATE`.
    pub bitrate: u32,
    /// Active DMA buffer (address and samples left).
    active: Option<DmaBuffer>,
    /// DMA buffers queued behind the active one.
    pending: std::collections::VecDeque<DmaBuffer>,
    /// One sample read ahead for linear interpolation.
    lookahead: Option<(i16, i16)>,
    /// DMA interrupt latch (the MI `AI` line).
    irq: bool,
    /// Host output sample rate.
    output_rate: f32,
    /// Resampler phase in source samples (`0.0..1.0` between `last` and the
    /// next queued sample).
    phase: f64,
    /// Last source sample used by the resampler.
    last: (i16, i16),
    /// Whether `last` holds a real sample.
    primed: bool,
    /// Whether `last` has been latched but not emitted yet.
    pending_first: bool,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            dram_addr: 0,
            len: 0,
            control: 0,
            dacrate: 0,
            bitrate: 0,
            active: None,
            pending: std::collections::VecDeque::new(),
            lookahead: None,
            irq: false,
            output_rate: 48_000.0,
            phase: 0.0,
            last: (0, 0),
            primed: false,
            pending_first: false,
        }
    }
}

impl Ai {
    /// Creates an idle AI.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// DAC sample rate implied by `AI_DACRATE`.
    ///
    /// The AI DAC clock is the 48.681818 MHz video clock.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        let divisor = u64::from(self.dacrate) + 1;
        let rate = 48_681_818u64 / divisor;
        if rate == 0 {
            48_000.0
        } else {
            rate as f32
        }
    }

    /// Host output sample rate.
    #[must_use]
    pub fn output_rate(&self) -> f32 {
        self.output_rate
    }

    /// Sets the host output sample rate used when resampling.
    pub fn set_output_rate(&mut self, rate: f32) {
        if rate.is_finite() && rate > 0.0 {
            self.output_rate = rate;
        }
    }

    /// Whether the AI DMA interrupt is pending.
    #[must_use]
    pub fn interrupt_pending(&self) -> bool {
        self.irq
    }

    /// Acknowledges the AI DMA interrupt (the MI handler clears it).
    pub fn clear_interrupt(&mut self) {
        self.irq = false;
    }

    fn status(&self) -> u32 {
        let queued = !self.pending.is_empty();
        let busy = self.active.is_some() || queued;
        // bit 0/31 = more than one buffer queued, bit 30 = DMA active,
        // bit 25 = DMA enabled, bits 20/24 are fixed ones.
        (u32::from(busy) << 30)
            | (u32::from(self.control & 1 != 0) << 25)
            | (1 << 24)
            | (1 << 20)
            | u32::from(queued)
            | (u32::from(queued) << 31)
    }

    /// Reads an AI register (`offset` from `0x0450_0000`).
    #[must_use]
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset & 0x1F {
            0x00 => self.dram_addr,
            0x04 => (self.active.map_or(0, |active| active.remaining as u32) * 4) & 0x3_FFFF,
            0x08 => self.control,
            0x0C => self.status(),
            0x10 => self.dacrate,
            0x14 => self.bitrate,
            _ => 0,
        }
    }

    /// Writes an AI register; `AI_LEN` queues a DMA buffer when control is
    /// enabled.
    pub fn write_register(&mut self, offset: u32, value: u32, rdram: &[u8]) {
        match offset & 0x1F {
            0x00 => self.dram_addr = value & 0x00FF_FFFF,
            0x04 => {
                self.len = value & 0x00FF_FFFF;
                self.queue_dma(rdram);
            }
            0x08 => self.control = value & 1,
            0x0C => self.clear_interrupt(),
            0x10 => self.dacrate = value & 0x3FFF,
            0x14 => self.bitrate = value & 0x7F,
            _ => {}
        }
    }

    fn queue_dma(&mut self, _rdram: &[u8]) {
        if self.control & 1 == 0 {
            return;
        }
        let samples = (self.len as usize & !7) / 4;
        if samples == 0 {
            return;
        }
        let buffer = DmaBuffer {
            address: self.dram_addr as usize,
            remaining: samples,
        };
        if self.active.is_none() && self.pending.is_empty() {
            // Starting the first buffer raises the AI interrupt so the CPU can
            // refill the queue.
            self.active = Some(buffer);
            self.irq = true;
        } else if self.pending.len() < 2 {
            self.pending.push_back(buffer);
        }
    }

    /// Reads the next source sample from RDRAM, advancing the DMA pointer and
    /// promoting the next queued buffer (raising the AI interrupt at the
    /// boundary) as the DAC clock consumes samples.
    fn read_sample(&mut self, rdram: &[u8]) -> Option<(i16, i16)> {
        loop {
            let Some(active) = self.active else {
                if let Some(next) = self.pending.pop_front() {
                    self.active = Some(next);
                    self.irq = true;
                    continue;
                }
                return None;
            };
            let offset = active.address;
            let remaining = active.remaining.saturating_sub(1);
            self.active = Some(DmaBuffer {
                address: offset + 4,
                remaining,
            });
            self.dram_addr = (offset + 4) as u32;
            let sample = rdram
                .get(offset..offset + 4)
                .map(|bytes| {
                    (
                        i16::from_be_bytes([bytes[0], bytes[1]]),
                        i16::from_be_bytes([bytes[2], bytes[3]]),
                    )
                })
                .unwrap_or((0, 0));
            if remaining == 0 {
                if let Some(next) = self.pending.pop_front() {
                    self.active = Some(next);
                    self.irq = true;
                } else {
                    self.active = None;
                }
            }
            return Some(sample);
        }
    }

    fn peek(&mut self, rdram: &[u8]) -> Option<(i16, i16)> {
        if self.lookahead.is_none() {
            self.lookahead = self.read_sample(rdram);
        }
        self.lookahead
    }

    fn take(&mut self, rdram: &[u8]) -> Option<(i16, i16)> {
        let sample = self.peek(rdram);
        self.lookahead = None;
        sample
    }

    /// Produces `count` host-rate stereo samples by linearly resampling the
    /// queued DMA stream. Underruns decay the last sample toward silence.
    pub fn generate(&mut self, count: usize, rdram: &[u8], out: &mut Vec<(i16, i16)>) {
        let source_rate = f64::from(self.sample_rate());
        let step = source_rate / f64::from(self.output_rate.max(1.0));
        for _ in 0..count {
            if !self.primed {
                if let Some(first) = self.take(rdram) {
                    self.last = first;
                    self.primed = true;
                    self.pending_first = true;
                } else {
                    out.push((0, 0));
                    continue;
                }
            }
            while self.phase >= 1.0 {
                if let Some(sample) = self.take(rdram) {
                    self.last = sample;
                    self.pending_first = true;
                } else {
                    self.phase = 1.0;
                    break;
                }
                self.phase -= 1.0;
            }
            let next = self.peek(rdram);
            if let Some(next) = next {
                self.pending_first = false;
                let frac = self.phase as f32;
                let left = self.last.0 as f32 + (next.0 as f32 - self.last.0 as f32) * frac;
                let right = self.last.1 as f32 + (next.1 as f32 - self.last.1 as f32) * frac;
                out.push((left.round() as i16, right.round() as i16));
            } else if self.pending_first {
                // Emit the final latched sample before decaying.
                self.pending_first = false;
                out.push(self.last);
            } else {
                // No data: decay toward silence like the DAC does.
                self.last = (decay(self.last.0), decay(self.last.1));
                out.push(self.last);
            }
            self.phase += step;
        }
    }
}

fn decay(value: i16) -> i16 {
    let scaled = f32::from(value) * 0.999;
    if scaled.abs() < 0.5 {
        0
    } else {
        scaled as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(samples: &[(i16, i16)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (left, right) in samples {
            bytes.extend_from_slice(&left.to_be_bytes());
            bytes.extend_from_slice(&right.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn resamples_and_decays_on_underrun() {
        let mut ai = Ai::new();
        ai.dacrate = 1013; // ~48.0 kHz source
        ai.control = 1;
        ai.dram_addr = 0;
        let mut ram = vec![0u8; 0x100];
        ram[..8].copy_from_slice(&stream(&[(1000, -1000), (2000, -2000)]));
        // Output at twice the source rate: linear interpolation hits the
        // midpoint between the two samples.
        ai.set_output_rate(ai.sample_rate() * 2.0);
        ai.write_register(0x04, 8, &ram);
        assert!(ai.interrupt_pending());
        let mut out = Vec::new();
        ai.generate(3, &ram, &mut out);
        assert_eq!(out[0], (1000, -1000));
        assert_eq!(out[1], (1500, -1500));
        assert!(out[2].0 >= 1990);
        // Once the source is exhausted the DAC decays toward silence.
        let mut tail = Vec::new();
        ai.generate(64, &ram, &mut tail);
        assert!(tail.last().unwrap().0.abs() < out[2].0.abs());
        ai.write_register(0x0C, 1, &ram);
        assert!(!ai.interrupt_pending());
    }

    #[test]
    fn dma_status_reflects_the_active_buffer() {
        let mut ai = Ai::new();
        ai.dram_addr = 0;
        ai.control = 1;
        let ram = vec![0u8; 0x40];
        ai.write_register(0x04, 8, &ram);
        // bit 30 = DMA active, bit 25 = enabled, bits 20/24 fixed.
        let status = ai.read_register(0x0C);
        assert_ne!(status & (1 << 30), 0);
        assert_ne!(status & (1 << 25), 0);
        assert_eq!(ai.read_register(0x04), 8);
    }
    #[test]
    fn dma_reads_on_demand_and_interrupts_at_boundaries() {
        let mut ai = Ai::new();
        ai.dacrate = 1013;
        ai.control = 1;
        ai.set_output_rate(ai.sample_rate()); // one source sample per output
        let mut ram = vec![0u8; 0x20];
        ram[..16].copy_from_slice(&stream(&[
            (100, -100),
            (200, -200),
            (300, -300),
            (400, -400),
        ]));
        ai.dram_addr = 0;
        ai.write_register(0x04, 8, &ram); // buffer 1
        assert!(ai.interrupt_pending());
        ai.write_register(0x0C, 1, &ram); // acknowledge
        ai.dram_addr = 8;
        ai.write_register(0x04, 8, &ram); // buffer 2
        assert!(!ai.interrupt_pending());

        // The DMA must read RDRAM when the DAC consumes the sample.
        ram[0..2].copy_from_slice(&2222i16.to_be_bytes());
        ram[2..4].copy_from_slice(&(-2222i16).to_be_bytes());
        let mut out = Vec::new();
        ai.generate(1, &ram, &mut out);
        assert_eq!(out[0], (2222, -2222));
        assert!(ai.interrupt_pending(), "buffer 1 completion raises AI IRQ");
        ai.write_register(0x0C, 1, &ram);

        out.clear();
        ai.generate(3, &ram, &mut out);
        assert_eq!(out, vec![(200, -200), (300, -300), (400, -400)]);
        assert!(
            !ai.interrupt_pending(),
            "no queued buffer after the last one"
        );
    }
}
