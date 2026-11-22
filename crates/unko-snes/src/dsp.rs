//! Sony S-DSP audio generator.
//!
//! The implementation follows the public S-DSP documentation and the
//! ISC-licensed ares `sfc/dsp` source. It is sample-driven: one call to
//! [`Dsp::run_sample`] executes the 32 internal DSP clocks that produce one
//! stereo frame at 32 kHz.

/// Number of DSP registers.
pub const DSP_REGISTERS: usize = 0x80;

/// S-DSP stereo sample.
pub type Sample = (i16, i16);

const COUNTER_RATE: [u16; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 40, 32,
    24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];

const COUNTER_OFFSET: [u16; 32] = [
    0, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040,
    536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 0, 0,
];

const GAUSSIAN_TABLE: [i16; 512] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2,
    2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10,
    10, 11, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 15, 16, 16, 17, 17, 18, 19, 19, 20, 20, 21, 21,
    22, 23, 23, 24, 24, 25, 26, 27, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 35, 36, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 58, 59, 60, 61, 62, 64, 65,
    66, 67, 69, 70, 71, 73, 74, 76, 77, 78, 80, 81, 83, 84, 86, 87, 89, 90, 92, 94, 95, 97, 99,
    100, 102, 104, 106, 107, 109, 111, 113, 115, 117, 118, 120, 122, 124, 126, 128, 130, 132, 134,
    137, 139, 141, 143, 145, 147, 150, 152, 154, 156, 159, 161, 163, 166, 168, 171, 173, 175, 178,
    180, 183, 186, 188, 191, 193, 196, 199, 201, 204, 207, 210, 212, 215, 218, 221, 224, 227, 230,
    233, 236, 239, 242, 245, 248, 251, 254, 257, 260, 263, 267, 270, 273, 276, 280, 283, 286, 290,
    293, 297, 300, 304, 307, 311, 314, 318, 321, 325, 328, 332, 336, 339, 343, 347, 351, 354, 358,
    362, 366, 370, 374, 378, 381, 385, 389, 393, 397, 401, 405, 410, 414, 418, 422, 426, 430, 434,
    439, 443, 447, 451, 456, 460, 464, 469, 473, 477, 482, 486, 491, 495, 499, 504, 508, 513, 517,
    522, 527, 531, 536, 540, 545, 550, 554, 559, 563, 568, 573, 577, 582, 587, 592, 596, 601, 606,
    611, 615, 620, 625, 630, 635, 640, 644, 649, 654, 659, 664, 669, 674, 678, 683, 688, 693, 698,
    703, 708, 713, 718, 723, 728, 732, 737, 742, 747, 752, 757, 762, 767, 772, 777, 782, 787, 792,
    797, 802, 806, 811, 816, 821, 826, 831, 836, 841, 846, 851, 855, 860, 865, 870, 875, 880, 884,
    889, 894, 899, 904, 908, 913, 918, 923, 927, 932, 937, 941, 946, 951, 955, 960, 965, 969, 974,
    978, 983, 988, 992, 997, 1001, 1005, 1010, 1014, 1019, 1023, 1027, 1032, 1036, 1040, 1045,
    1049, 1053, 1057, 1061, 1066, 1070, 1074, 1078, 1082, 1086, 1090, 1094, 1098, 1102, 1106, 1109,
    1113, 1117, 1121, 1125, 1128, 1132, 1136, 1139, 1143, 1146, 1150, 1153, 1157, 1160, 1164, 1167,
    1170, 1174, 1177, 1180, 1183, 1186, 1190, 1193, 1196, 1199, 1202, 1205, 1207, 1210, 1213, 1216,
    1219, 1221, 1224, 1227, 1229, 1232, 1234, 1237, 1239, 1241, 1244, 1246, 1248, 1251, 1253, 1255,
    1257, 1259, 1261, 1263, 1265, 1267, 1269, 1270, 1272, 1274, 1275, 1277, 1279, 1280, 1282, 1283,
    1284, 1286, 1287, 1288, 1290, 1291, 1292, 1293, 1294, 1295, 1296, 1297, 1297, 1298, 1299, 1300,
    1300, 1301, 1302, 1302, 1303, 1303, 1303, 1304, 1304, 1304, 1304, 1304, 1305, 1305,
];

#[derive(
    serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
)]
enum EnvelopeMode {
    Release,
    Attack,
    Decay,
    Sustain,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Clock {
    counter: u16,
    sample: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct MainVol {
    reset: bool,
    mute: bool,
    volume: [i8; 2],
    output: [i32; 2],
}

impl Default for MainVol {
    fn default() -> Self {
        // ares `MainVol { n1 reset = 1; n1 mute = 1; }`: the DSP powers up
        // muted and in reset until the sound driver writes `$6C`.
        Self {
            reset: true,
            mute: true,
            volume: [0; 2],
            output: [0; 2],
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Echo {
    feedback: i8,
    volume: [i8; 2],
    fir: [i8; 8],
    history: [[i16; 8]; 2],
    page: u8,
    delay: u8,
    readonly: bool,
    input: [i32; 2],
    output: [i32; 2],
    _page: u8,
    _readonly: bool,
    _address: u16,
    _offset: u16,
    _length: u16,
    _history_offset: u8,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            feedback: 0,
            volume: [0; 2],
            fir: [0; 8],
            history: [[0; 8]; 2],
            page: 0,
            delay: 0,
            readonly: true,
            input: [0; 2],
            output: [0; 2],
            _page: 0,
            _readonly: true,
            _address: 0,
            _offset: 0,
            _length: 0,
            _history_offset: 0,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Noise {
    frequency: u8,
    lfsr: u16,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
struct Brr {
    bank: u8,
    _bank: u8,
    _source: u8,
    _address: u16,
    _next_address: u16,
    _header: u8,
    _byte: u8,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
struct Latch {
    adsr0: u8,
    envx: u8,
    outx: u8,
    pitch: u16,
    output: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Voice {
    index: u8,
    volume: [i8; 2],
    pitch: u16,
    source: u8,
    adsr0: u8,
    adsr1: u8,
    gain: u8,
    envx: u8,
    keyon: bool,
    keyoff: bool,
    modulate: bool,
    noise: bool,
    echo: bool,
    end: bool,
    buffer: [i16; 12],
    buffer_offset: usize,
    gaussian_offset: u16,
    brr_address: u16,
    brr_offset: usize,
    keyon_delay: u8,
    envelope_mode: EnvelopeMode,
    envelope: u16,
    _envelope: i32,
    _keylatch: bool,
    _keyon: bool,
    _keyoff: bool,
    _modulate: bool,
    _noise: bool,
    _echo: bool,
    _end: bool,
    _looped: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            index: 0,
            volume: [0; 2],
            pitch: 0,
            source: 0,
            adsr0: 0,
            adsr1: 0,
            gain: 0,
            envx: 0,
            keyon: false,
            keyoff: false,
            modulate: false,
            noise: false,
            echo: false,
            end: false,
            buffer: [0; 12],
            buffer_offset: 0,
            gaussian_offset: 0,
            brr_address: 0,
            brr_offset: 1,
            keyon_delay: 0,
            envelope_mode: EnvelopeMode::Release,
            envelope: 0,
            _envelope: 0,
            _keylatch: false,
            _keyon: false,
            _keyoff: false,
            _modulate: false,
            _noise: false,
            _echo: false,
            _end: false,
            _looped: false,
        }
    }
}

/// The S-DSP.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Dsp {
    /// Raw register file, `$00-$7F`.
    #[serde(with = "serde_big_array::BigArray")]
    pub registers: [u8; DSP_REGISTERS],
    clock: Clock,
    mainvol: MainVol,
    echo: Echo,
    noise: Noise,
    brr: Brr,
    latch: Latch,
    voice: [Voice; 8],
    /// Index of the next micro-step in the 32-step sample sequence.
    step: u8,
    sample_out: Sample,
    /// Number of finished output samples since reset (debug).
    samples_run: u64,
    /// Optional bounded debug trace (see [`DspTrace`]).
    pub trace: Option<DspTrace>,
    /// Optional all-voice per-sample trace used to diff the DSP pipeline.
    #[serde(skip)]
    pub full_trace: Option<std::collections::VecDeque<[i32; 204]>>,
    /// S-SMP PC of the instruction issuing the current register write (debug).
    #[serde(skip)]
    pub debug_pc: u16,
    /// S-SMP clock at the current register write (debug).
    #[serde(skip)]
    pub debug_clock: u128,
}

/// Bounded S-DSP debug trace used to diff the voice pipeline against another
/// emulator. Rows are one entry per finished output sample, writes one entry
/// per S-SMP DSP register write.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DspTrace {
    /// One row per finished output sample: `[sample, keyon_delay, envelope, envx,
    /// brr_address, brr_offset, gaussian_offset, latch_pitch, header, byte,
    /// latch_output, voice_pitch]` for voice 0.
    pub rows: std::collections::VecDeque<[i32; 12]>,
    /// `(sample, register, value, micro-step, S-SMP PC, S-SMP clock)` per write.
    pub writes: std::collections::VecDeque<(u64, u8, u8, u64, u16, u128)>,
}

impl Default for Dsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Dsp {
    /// Create a powered-on DSP.
    pub fn new() -> Self {
        let mut voice: [Voice; 8] = std::array::from_fn(|_| Voice::default());
        for (index, voice) in voice.iter_mut().enumerate() {
            voice.index = (index as u8) << 4;
        }
        Self {
            registers: [0; DSP_REGISTERS],
            clock: Clock {
                counter: 0,
                // ares `Clock { n1 sample = 1; }`: the key-on poll is sampled on
                // the *second* output sample, so this must start true.
                sample: true,
            },
            mainvol: MainVol::default(),
            echo: Echo::default(),
            noise: Noise {
                frequency: 0,
                lfsr: 0x4000,
            },
            brr: Brr::default(),
            latch: Latch::default(),
            voice,
            step: 0,
            sample_out: (0, 0),
            samples_run: 0,
            trace: None,
            full_trace: None,
            debug_pc: 0,
            debug_clock: 0,
        }
    }

    /// Record the S-SMP PC/clock of the instruction issuing the next write.
    pub fn set_debug_pc(&mut self, pc: u16) {
        self.debug_pc = pc;
    }

    /// Record the S-SMP clock at the next register write (debug).
    pub fn set_debug_clock(&mut self, clock: u128) {
        self.debug_clock = clock;
    }

    /// Start recording the bounded debug trace.
    pub fn enable_trace(&mut self) {
        if self.trace.is_none() {
            self.trace = Some(DspTrace::default());
        }
    }

    /// Start recording every voice's state once per output sample.
    pub fn enable_full_trace(&mut self) {
        if self.full_trace.is_none() {
            self.full_trace = Some(std::collections::VecDeque::new());
        }
    }

    /// Reset the DSP's decoded state.
    ///
    /// ares `DSP::power(reset = true)` clears the voice/echo/noise state but
    /// leaves the raw `registers[]` file (and the power-on random pattern)
    /// intact, so a soft reset must not zero it here either.
    pub fn reset(&mut self) {
        let registers = self.registers;
        self.step = 0;
        *self = Self::new();
        self.registers = registers;
    }

    /// Whether the global mute flag (`$6C` bit 6) is set.
    pub const fn muted(&self) -> bool {
        self.mainvol.mute
    }

    /// Read a DSP register.
    pub fn read(&self, address: u8) -> u8 {
        self.registers[usize::from(address & 0x7F)]
    }

    /// Write a DSP register.
    pub fn write(&mut self, address: u8, data: u8) {
        // ares `SMP::writeIO` case 0xf3: "0x80-0xff are read-only mirrors of
        // 0x00-0x7f", so a write with bit 7 set is dropped. Reads alias.
        if address >= 0x80 {
            return;
        }
        self.registers[usize::from(address)] = data;
        if let Some(trace) = &mut self.trace {
            trace.writes.push_back((
                self.samples_run,
                address,
                data,
                self.samples_run * 32 + u64::from(self.step),
                self.debug_pc,
                self.debug_clock,
            ));
            if trace.writes.len() > 2_000_000 {
                trace.writes.pop_front();
            }
        }

        match address {
            0x0C => self.mainvol.volume[0] = data as i8,
            0x1C => self.mainvol.volume[1] = data as i8,
            0x2C => self.echo.volume[0] = data as i8,
            0x3C => self.echo.volume[1] = data as i8,
            0x4C => {
                for (index, voice) in self.voice.iter_mut().enumerate() {
                    let set = data & (1 << index) != 0;
                    voice.keyon = set;
                    voice._keylatch = set;
                }
            }
            0x5C => {
                for (index, voice) in self.voice.iter_mut().enumerate() {
                    voice.keyoff = data & (1 << index) != 0;
                }
            }
            0x6C => {
                self.noise.frequency = data & 0x1F;
                self.echo.readonly = data & 0x20 != 0;
                self.mainvol.mute = data & 0x40 != 0;
                self.mainvol.reset = data & 0x80 != 0;
            }
            0x7C => {
                for voice in &mut self.voice {
                    voice._end = false;
                }
                self.registers[0x7C] = 0;
            }
            0x0D => self.echo.feedback = data as i8,
            0x2D => {
                for (index, voice) in self.voice.iter_mut().enumerate() {
                    voice.modulate = data & (1 << index) != 0;
                }
                self.voice[0].modulate = false;
            }
            0x3D => {
                for (index, voice) in self.voice.iter_mut().enumerate() {
                    voice.noise = data & (1 << index) != 0;
                }
            }
            0x4D => {
                for (index, voice) in self.voice.iter_mut().enumerate() {
                    voice.echo = data & (1 << index) != 0;
                }
            }
            0x5D => self.brr.bank = data,
            0x6D => self.echo.page = data,
            0x7D => self.echo.delay = data & 0x0F,
            _ => {}
        }

        let voice_index = usize::from((address >> 4) & 0x07);
        match address & 0x0F {
            0x00 => self.voice[voice_index].volume[0] = data as i8,
            0x01 => self.voice[voice_index].volume[1] = data as i8,
            0x02 => {
                self.voice[voice_index].pitch =
                    (self.voice[voice_index].pitch & 0x3F00) | u16::from(data);
            }
            0x03 => {
                self.voice[voice_index].pitch =
                    (self.voice[voice_index].pitch & 0x00FF) | (u16::from(data & 0x3F) << 8);
            }
            0x04 => self.voice[voice_index].source = data,
            0x05 => self.voice[voice_index].adsr0 = data,
            0x06 => self.voice[voice_index].adsr1 = data,
            0x07 => self.voice[voice_index].gain = data,
            0x08 => self.latch.envx = data,
            0x09 => self.latch.outx = data,
            0x0F => self.echo.fir[voice_index] = data as i8,
            _ => {}
        }
    }

    /// Run one 32 kHz stereo sample through the DSP.
    /// Advance the S-DSP by one micro-step, returning the finished sample when
    /// the 32-step sequence wraps.
    pub fn tick(&mut self, ram: &mut [u8]) -> Option<Sample> {
        self.step_once(ram);
        self.step = (self.step + 1) & 0x1F;
        if self.step == 0 {
            self.samples_run += 1;
            if let Some(trace) = &mut self.trace {
                let voice = &self.voice[0];
                let row = [
                    self.samples_run as i32,
                    i32::from(voice.keyon_delay),
                    i32::from(voice.envelope),
                    i32::from(voice.envx),
                    i32::from(voice.brr_address),
                    voice.brr_offset as i32,
                    i32::from(voice.gaussian_offset),
                    i32::from(self.latch.pitch),
                    i32::from(self.brr._header),
                    i32::from(self.brr._byte),
                    self.latch.output,
                    i32::from(voice.pitch),
                ];
                trace.rows.push_back(row);
                if trace.rows.len() > 400_000 {
                    trace.rows.pop_front();
                }
            }
            Some(self.sample_out)
        } else {
            None
        }
    }

    /// Run a whole sample (32 ticks). Used by the unit tests.
    pub fn run_sample(&mut self, ram: &mut [u8]) -> Sample {
        let mut sample = self.sample_out;
        for _ in 0..32 {
            if let Some(done) = self.tick(ram) {
                sample = done;
            }
        }
        sample
    }

    /// Run one of the 32 S-DSP micro-steps that make up a sample.
    ///
    /// ares' DSP thread steps `3 * 8` apu clocks per tick and is synchronized
    /// with the S-SMP on every access, so exactly one tick elapses per S-SMP
    /// access unit (2 clocks). Running the whole 32-step sample at once would
    /// hide the echo buffer's partial state from an S-SMP read that lands
    /// mid-sample, which is what blargg's echo ordering tests probe.
    fn step_once(&mut self, ram: &mut [u8]) {
        match self.step {
            0 => {
                self.voice5(ram, 0);
                self.voice2(ram, 1);
            }
            1 => {
                self.voice6(ram, 0);
                self.voice3(ram, 1);
            }
            2 => {
                self.voice7(ram, 0);
                self.voice4(ram, 1);
                self.voice1(ram, 3);
            }
            3 => {
                self.voice8(ram, 0);
                self.voice5(ram, 1);
                self.voice2(ram, 2);
            }
            4 => {
                self.voice9(ram, 0);
                self.voice6(ram, 1);
                self.voice3(ram, 2);
            }
            5 => {
                self.voice7(ram, 1);
                self.voice4(ram, 2);
                self.voice1(ram, 4);
            }
            6 => {
                self.voice8(ram, 1);
                self.voice5(ram, 2);
                self.voice2(ram, 3);
            }
            7 => {
                self.voice9(ram, 1);
                self.voice6(ram, 2);
                self.voice3(ram, 3);
            }
            8 => {
                self.voice7(ram, 2);
                self.voice4(ram, 3);
                self.voice1(ram, 5);
            }
            9 => {
                self.voice8(ram, 2);
                self.voice5(ram, 3);
                self.voice2(ram, 4);
            }
            10 => {
                self.voice9(ram, 2);
                self.voice6(ram, 3);
                self.voice3(ram, 4);
            }
            11 => {
                self.voice7(ram, 3);
                self.voice4(ram, 4);
                self.voice1(ram, 6);
            }
            12 => {
                self.voice8(ram, 3);
                self.voice5(ram, 4);
                self.voice2(ram, 5);
            }
            13 => {
                self.voice9(ram, 3);
                self.voice6(ram, 4);
                self.voice3(ram, 5);
            }
            14 => {
                self.voice7(ram, 4);
                self.voice4(ram, 5);
                self.voice1(ram, 7);
            }
            15 => {
                self.voice8(ram, 4);
                self.voice5(ram, 5);
                self.voice2(ram, 6);
            }
            16 => {
                self.voice9(ram, 4);
                self.voice6(ram, 5);
                self.voice3(ram, 6);
            }
            17 => {
                self.voice1(ram, 0);
                self.voice7(ram, 5);
                self.voice4(ram, 6);
            }
            18 => {
                self.voice8(ram, 5);
                self.voice5(ram, 6);
                self.voice2(ram, 7);
            }
            19 => {
                self.voice9(ram, 5);
                self.voice6(ram, 6);
                self.voice3(ram, 7);
            }
            20 => {
                self.voice1(ram, 1);
                self.voice7(ram, 6);
                self.voice4(ram, 7);
            }
            21 => {
                self.voice8(ram, 6);
                self.voice5(ram, 7);
                self.voice2(ram, 0);
            }
            22 => {
                self.voice3a(ram, 0);
                self.voice9(ram, 6);
                self.voice6(ram, 7);
                self.echo22(ram);
            }
            23 => {
                self.voice7(ram, 7);
                self.echo23(ram);
            }
            24 => {
                self.voice8(ram, 7);
                self.echo24(ram);
            }
            25 => {
                self.voice3b(ram, 0);
                self.voice9(ram, 7);
                self.echo25(ram);
            }
            26 => {
                self.echo26(ram);
            }
            27 => {
                self.misc27();
                self.echo27();
            }
            28 => {
                self.misc28();
                self.echo28();
            }
            29 => {
                self.misc29();
                self.echo29(ram);
            }
            30 => {
                self.misc30();
                self.voice3c(ram, 0);
                self.echo30(ram);
            }
            31 => {
                self.voice4(ram, 0);
                self.voice1(ram, 2);
            }
            _ => unreachable!(),
        }
    }

    fn voice1(&mut self, _ram: &mut [u8], index: usize) {
        // 16-bit sample address; the sum wraps in hardware (ares `n16`).
        self.brr._address =
            (u16::from(self.brr._bank) << 8).wrapping_add(u16::from(self.brr._source) << 2);
        self.brr._source = self.voice[index].source;
    }

    fn voice2(&mut self, ram: &mut [u8], index: usize) {
        let mut address = self.brr._address;
        if self.voice[index].keyon_delay == 0 {
            address = address.wrapping_add(2);
        }
        let low = ram[usize::from(address)];
        let high = ram[usize::from(address.wrapping_add(1))];
        self.brr._next_address = u16::from_le_bytes([low, high]);
        self.latch.adsr0 = self.voice[index].adsr0;
        self.latch.pitch = self.voice[index].pitch & 0x00FF;
    }

    fn voice3(&mut self, ram: &mut [u8], index: usize) {
        self.voice3a(ram, index);
        self.voice3b(ram, index);
        self.voice3c(ram, index);
    }

    fn voice3a(&mut self, _ram: &mut [u8], index: usize) {
        self.latch.pitch |= self.voice[index].pitch & 0xFF00;
    }

    fn voice3b(&mut self, ram: &mut [u8], index: usize) {
        let voice = &self.voice[index];
        self.brr._byte = ram[usize::from(voice.brr_address.wrapping_add(voice.brr_offset as u16))];
        self.brr._header = ram[usize::from(voice.brr_address)];
    }

    fn voice3c(&mut self, ram: &mut [u8], index: usize) {
        if self.voice[index]._modulate {
            self.latch.pitch = self.latch.pitch.wrapping_add(
                (((self.latch.output >> 5) * i32::from(self.latch.pitch)) >> 10) as u16,
            );
        }

        if self.voice[index].keyon_delay != 0 {
            if self.voice[index].keyon_delay == 5 {
                self.voice[index].brr_address = self.brr._next_address;
                self.voice[index].brr_offset = 1;
                self.voice[index].buffer_offset = 0;
                self.brr._header = 0;
            }
            self.voice[index].envelope = 0;
            self.voice[index]._envelope = 0;
            self.voice[index].gaussian_offset = 0;
            self.voice[index].keyon_delay = self.voice[index].keyon_delay.wrapping_sub(1);
            if (self.voice[index].keyon_delay & 3) != 0 {
                self.voice[index].gaussian_offset = 0x4000;
            }
            self.latch.pitch = 0;
        }

        let mut output = self.gaussian_interpolate(index);

        if self.voice[index]._noise {
            output = i32::from((self.noise.lfsr << 1) as i16);
        }

        self.latch.output = ((output * i32::from(self.voice[index].envelope)) >> 11) & !1;
        self.voice[index].envx = (self.voice[index].envelope >> 4) as u8;

        if self.mainvol.reset || (self.brr._header & 0x03) == 1 {
            self.voice[index].envelope_mode = EnvelopeMode::Release;
            self.voice[index].envelope = 0;
        }

        if self.clock.sample {
            if self.voice[index]._keyoff {
                self.voice[index].envelope_mode = EnvelopeMode::Release;
            }
            if self.voice[index]._keyon {
                self.voice[index].keyon_delay = 5;
                self.voice[index].envelope_mode = EnvelopeMode::Attack;
            }
        }

        if self.voice[index].keyon_delay == 0 {
            self.envelope_run(index);
        }
        let _ = ram;
    }

    fn voice4(&mut self, ram: &mut [u8], index: usize) {
        self.voice[index]._looped = false;
        if self.voice[index].gaussian_offset >= 0x4000 {
            self.brr_decode(ram, index);
            self.voice[index].brr_offset = self.voice[index].brr_offset.wrapping_add(2);
            if self.voice[index].brr_offset >= 9 {
                self.voice[index].brr_address = self.voice[index].brr_address.wrapping_add(9);
                if (self.brr._header & 0x01) != 0 {
                    self.voice[index].brr_address = self.brr._next_address;
                    self.voice[index]._looped = true;
                }
                self.voice[index].brr_offset = 1;
            }
        }

        self.voice[index].gaussian_offset =
            (self.voice[index].gaussian_offset & 0x3FFF).wrapping_add(self.latch.pitch);
        if self.voice[index].gaussian_offset > 0x7FFF {
            self.voice[index].gaussian_offset = 0x7FFF;
        }
        self.voice_output(index, 0);
    }

    fn voice5(&mut self, _ram: &mut [u8], index: usize) {
        self.voice_output(index, 1);
        self.voice[index]._end |= self.voice[index]._looped;
        if self.voice[index].keyon_delay == 5 {
            self.voice[index]._end = false;
        }
    }

    fn voice6(&mut self, _ram: &mut [u8], index: usize) {
        let _ = index;
        self.latch.outx = (self.latch.output >> 8) as u8;
    }

    fn voice7(&mut self, _ram: &mut [u8], index: usize) {
        for voice in 0..8 {
            self.registers[0x7C] = (self.registers[0x7C] & !(1 << voice))
                | (u8::from(self.voice[voice]._end) << voice);
        }
        self.latch.envx = self.voice[index].envx;
    }

    fn voice8(&mut self, _ram: &mut [u8], index: usize) {
        self.registers[usize::from(self.voice[index].index | 0x09)] = self.latch.outx;
    }

    fn voice9(&mut self, _ram: &mut [u8], index: usize) {
        self.registers[usize::from(self.voice[index].index | 0x08)] = self.latch.envx;
    }

    fn voice_output(&mut self, index: usize, channel: usize) {
        let amp = (self.latch.output * i32::from(self.voice[index].volume[channel])) >> 7;
        self.mainvol.output[channel] = clamp16(self.mainvol.output[channel] + amp);
        if self.voice[index]._echo {
            self.echo.output[channel] = clamp16(self.echo.output[channel] + amp);
        }
    }

    fn envelope_run(&mut self, index: usize) {
        let mut envelope = i32::from(self.voice[index].envelope);
        let mode = self.voice[index].envelope_mode;

        if mode == EnvelopeMode::Release {
            envelope -= 0x08;
            if envelope < 0 {
                envelope = 0;
            }
            self.voice[index].envelope = envelope as u16;
            return;
        }

        let mut envelope_data = self.voice[index].adsr1;
        let mut rate;
        if (self.latch.adsr0 & 0x80) != 0 {
            if mode >= EnvelopeMode::Decay {
                envelope -= 1;
                envelope -= envelope >> 8;
                rate = envelope_data & 0x1F;
                if mode == EnvelopeMode::Decay {
                    rate = ((self.latch.adsr0 >> 4) & 0x07) * 2 + 16;
                }
            } else {
                rate = (self.latch.adsr0 & 0x0F) * 2 + 1;
                envelope += if rate < 31 { 0x20 } else { 0x400 };
            }
        } else {
            envelope_data = self.voice[index].gain;
            let mode = envelope_data >> 5;
            if mode < 4 {
                envelope = i32::from(envelope_data) << 4;
                rate = 31;
            } else {
                rate = envelope_data & 0x1F;
                if mode == 4 {
                    envelope -= 0x20;
                } else if mode < 6 {
                    envelope -= 1;
                    envelope -= envelope >> 8;
                } else {
                    envelope += 0x20;
                    // GAIN mode 7 (two-slope linear increase): the slope halves once
                    // the internal envelope latch reaches 0x600. ares compares the
                    // latch as `(u32)v._envelope`, so a negative latch (left by a
                    // mode 4/5 decrease that underflowed) still satisfies the
                    // threshold. A signed compare misses that case and
                    // over-increments (blargg `spc_dsp6` `Envelope/gain $E0
                    // threshold`).
                    if mode > 6 && (self.voice[index]._envelope as u32) >= 0x600 {
                        envelope += 0x08 - 0x20;
                    }
                }
            }
        }

        if (envelope >> 8) == i32::from(envelope_data >> 5)
            && self.voice[index].envelope_mode == EnvelopeMode::Decay
        {
            self.voice[index].envelope_mode = EnvelopeMode::Sustain;
        }
        self.voice[index]._envelope = envelope;

        if envelope as u32 > 0x7FF {
            envelope = if envelope < 0 { 0 } else { 0x7FF };
            if self.voice[index].envelope_mode == EnvelopeMode::Attack {
                self.voice[index].envelope_mode = EnvelopeMode::Decay;
            }
        }

        if self.counter_poll(rate as u32) {
            self.voice[index].envelope = envelope as u16;
        }
    }

    fn brr_decode(&mut self, ram: &mut [u8], index: usize) {
        let voice = &self.voice[index];
        let next = ram[usize::from(
            voice
                .brr_address
                .wrapping_add(voice.brr_offset as u16)
                .wrapping_add(1),
        )];
        let mut nybbles = (i32::from(self.brr._byte) << 8) | i32::from(next);
        let filter = (self.brr._header >> 2) & 0x03;
        let scale = self.brr._header >> 4;

        for _ in 0..4 {
            let mut sample = (nybbles as i16 >> 12) as i32;
            nybbles <<= 4;

            if scale <= 12 {
                sample <<= scale;
                sample >>= 1;
            } else {
                sample &= !0x07FF;
            }

            let mut offset = self.voice[index].buffer_offset as i32;
            offset -= 1;
            if offset < 0 {
                offset = 11;
            }
            let p1 = i32::from(self.voice[index].buffer[offset as usize]);
            offset -= 1;
            if offset < 0 {
                offset = 11;
            }
            let p2 = i32::from(self.voice[index].buffer[offset as usize]) >> 1;

            match filter {
                0 => {}
                1 => {
                    sample += p1 >> 1;
                    sample += (-p1) >> 5;
                }
                2 => {
                    sample += p1;
                    sample -= p2;
                    sample += p2 >> 4;
                    sample += (p1 * -3) >> 6;
                }
                3 => {
                    sample += p1;
                    sample -= p2;
                    sample += (p1 * -13) >> 7;
                    sample += (p2 * 3) >> 4;
                }
                _ => unreachable!(),
            }

            sample = clamp16(sample);
            let sample = (sample << 1) as i16;
            self.voice[index].buffer[self.voice[index].buffer_offset] = sample;
            self.voice[index].buffer_offset += 1;
            if self.voice[index].buffer_offset >= 12 {
                self.voice[index].buffer_offset = 0;
            }
        }
    }

    fn gaussian_interpolate(&self, index: usize) -> i32 {
        let voice = &self.voice[index];
        let mut offset = (voice.gaussian_offset >> 4) as u8 as usize;
        let forward = 255 - offset;
        let reverse = offset;

        offset = (voice.buffer_offset + usize::from(voice.gaussian_offset >> 12)) % 12;
        let mut output =
            (i32::from(GAUSSIAN_TABLE[forward]) * i32::from(voice.buffer[offset])) >> 11;
        if offset + 1 >= 12 {
            offset = 0;
        } else {
            offset += 1;
        }
        output +=
            (i32::from(GAUSSIAN_TABLE[forward + 256]) * i32::from(voice.buffer[offset])) >> 11;
        if offset + 1 >= 12 {
            offset = 0;
        } else {
            offset += 1;
        }
        output +=
            (i32::from(GAUSSIAN_TABLE[reverse + 256]) * i32::from(voice.buffer[offset])) >> 11;
        if offset + 1 >= 12 {
            offset = 0;
        } else {
            offset += 1;
        }
        output = output as i16 as i32;
        output += (i32::from(GAUSSIAN_TABLE[reverse]) * i32::from(voice.buffer[offset])) >> 11;
        clamp16(output) & !1
    }

    fn counter_tick(&mut self) {
        if self.clock.counter == 0 {
            self.clock.counter = 2048 * 5 * 3;
        }
        self.clock.counter = self.clock.counter.wrapping_sub(1);
    }

    fn counter_poll(&self, rate: u32) -> bool {
        if rate == 0 {
            return false;
        }
        (u32::from(self.clock.counter) + u32::from(COUNTER_OFFSET[rate as usize]))
            % u32::from(COUNTER_RATE[rate as usize])
            == 0
    }

    fn calculate_fir(&self, channel: usize, index: usize) -> i32 {
        let history_index = (self.echo._history_offset as usize + index + 1) & 7;
        (i32::from(self.echo.history[channel][history_index]) * i32::from(self.echo.fir[index]))
            >> 6
    }

    fn echo_output(&self, channel: usize) -> i16 {
        // ares declares both shifted products as `i16` locals, so each is
        // truncated to 16 bits before the sum (RustySNES mirrors this).
        let main =
            ((self.mainvol.output[channel] * i32::from(self.mainvol.volume[channel])) >> 7) as i16;
        let echo = ((self.echo.input[channel] * i32::from(self.echo.volume[channel])) >> 7) as i16;
        clamp16(i32::from(main) + i32::from(echo)) as i16
    }

    fn echo_read(&mut self, ram: &mut [u8], channel: usize) {
        let mut address = self.echo._address.wrapping_add((channel as u16) * 2);
        let low = ram[usize::from(address)];
        address = address.wrapping_add(1);
        let high = ram[usize::from(address)];
        let sample = i16::from_le_bytes([low, high]);
        self.echo.history[channel][usize::from(self.echo._history_offset)] = sample >> 1;
    }

    fn echo_write(&mut self, ram: &mut [u8], channel: usize) {
        if !self.echo._readonly {
            let mut address = self.echo._address.wrapping_add((channel as u16) * 2);
            let sample = self.echo.output[channel] as i16;
            let [low, high] = sample.to_le_bytes();
            ram[usize::from(address)] = low;
            address = address.wrapping_add(1);
            ram[usize::from(address)] = high;
        }
        self.echo.output[channel] = 0;
    }

    fn echo22(&mut self, ram: &mut [u8]) {
        self.echo._history_offset = (self.echo._history_offset + 1) & 0x07;
        // The echo pointer is a 16-bit address and wraps on overflow.
        self.echo._address = (u16::from(self.echo._page) << 8).wrapping_add(self.echo._offset);
        self.echo_read(ram, 0);
        self.echo.input[0] = self.calculate_fir(0, 0);
        self.echo.input[1] = self.calculate_fir(1, 0);
    }

    fn echo23(&mut self, ram: &mut [u8]) {
        self.echo.input[0] += self.calculate_fir(0, 1) + self.calculate_fir(0, 2);
        self.echo.input[1] += self.calculate_fir(1, 1) + self.calculate_fir(1, 2);
        self.echo_read(ram, 1);
    }

    fn echo24(&mut self, _ram: &mut [u8]) {
        self.echo.input[0] +=
            self.calculate_fir(0, 3) + self.calculate_fir(0, 4) + self.calculate_fir(0, 5);
        self.echo.input[1] +=
            self.calculate_fir(1, 3) + self.calculate_fir(1, 4) + self.calculate_fir(1, 5);
    }

    fn echo25(&mut self, _ram: &mut [u8]) {
        let mut left = self.echo.input[0] + self.calculate_fir(0, 6);
        let mut right = self.echo.input[1] + self.calculate_fir(1, 6);
        left = left as i16 as i32;
        right = right as i16 as i32;
        left += self.calculate_fir(0, 7) as i16 as i32;
        right += self.calculate_fir(1, 7) as i16 as i32;
        self.echo.input[0] = clamp16(left) & !1;
        self.echo.input[1] = clamp16(right) & !1;
    }

    fn echo26(&mut self, _ram: &mut [u8]) {
        self.mainvol.output[0] = i32::from(self.echo_output(0));
        let left = self.echo.output[0]
            + i32::from(((self.echo.input[0] * i32::from(self.echo.feedback)) >> 7) as i16);
        let right = self.echo.output[1]
            + i32::from(((self.echo.input[1] * i32::from(self.echo.feedback)) >> 7) as i16);
        self.echo.output[0] = clamp16(left) & !1;
        self.echo.output[1] = clamp16(right) & !1;
    }

    fn echo27(&mut self) {
        let mut left = self.mainvol.output[0];
        let mut right = self.echo_output(1);
        self.mainvol.output[0] = 0;
        self.mainvol.output[1] = 0;
        if self.mainvol.mute {
            left = 0;
            right = 0;
        }
        self.sample_out = (left as i16, right);
        if let (Ok(path), Ok(target)) = (
            std::env::var("SNES_DSP_STATE"),
            std::env::var("SNES_DSP_STATE_SAMPLE"),
        ) {
            let from = target.parse::<u64>().unwrap_or(u64::MAX);
            let to = std::env::var("SNES_DSP_STATE_TO")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(from);
            if self.samples_run >= from && self.samples_run <= to {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(
                        f,
                        "sample {} left {} right {}",
                        self.samples_run, left as i16, right
                    );
                    for (index, voice) in self.voice.iter().enumerate() {
                        let _ = writeln!(
                            f,
                            "v{index} envx={} env={} pitch={} brr={:04X} off={} gauss={} kdelay={} mode={} src={:02X}",
                            voice.envx,
                            voice.envelope,
                            voice.pitch,
                            voice.brr_address,
                            voice.brr_offset,
                            voice.gaussian_offset,
                            voice.keyon_delay,
                            voice.envelope_mode as i32,
                            voice.source
                        );
                    }
                }
            }
        }
        // ares emits the DAC sample from `echo27`, so a full trace must be
        // captured here (not at the end of the 32-step cycle) to compare the
        // voice pipeline at the same point.
        if let Some(trace) = &mut self.full_trace {
            let mut row = [0i32; 204];
            row[0] = self.samples_run as i32;
            row[1] = i32::from(left as i16);
            row[2] = i32::from(right);
            for (index, voice) in self.voice.iter().enumerate() {
                let base = 3 + index * 21;
                row[base] = i32::from(voice.envx);
                row[base + 1] = i32::from(voice.envelope);
                row[base + 2] = i32::from(voice.pitch);
                row[base + 3] = i32::from(voice.brr_address);
                row[base + 4] = voice.brr_offset as i32;
                row[base + 5] = i32::from(voice.gaussian_offset);
                row[base + 6] = i32::from(voice.keyon_delay);
                row[base + 7] = voice.envelope_mode as i32;
                for (n, sample) in voice.buffer.iter().enumerate() {
                    row[base + 8 + n] = i32::from(*sample);
                }
                row[base + 20] = i32::from(voice._keyoff);
            }
            row[171] = i32::from(self.brr._header);
            row[172] = i32::from(self.brr._byte);
            row[173] = i32::from(self.mainvol.volume[0]);
            row[174] = i32::from(self.mainvol.volume[1]);
            row[175] = self.mainvol.output[0];
            row[176] = self.mainvol.output[1];
            row[177] = i32::from(self.echo.feedback);
            row[178] = i32::from(self.echo.volume[0]);
            row[179] = i32::from(self.echo.volume[1]);
            row[180] = self.echo.input[0];
            row[181] = self.echo.input[1];
            row[182] = self.echo.output[0];
            row[183] = self.echo.output[1];
            row[184] = i32::from(self.echo.page);
            row[185] = i32::from(self.echo.delay);
            row[186] = i32::from(self.echo._page);
            row[187] = i32::from(self.echo._offset);
            row[188] = i32::from(self.echo._length);
            row[189] = i32::from(self.echo._history_offset);
            row[190] = i32::from(self.noise.frequency);
            row[191] = i32::from(self.noise.lfsr);
            row[192] = self.latch.output;
            row[193] = i32::from(self.echo._readonly);
            row[194] = i32::from(self.mainvol.mute);
            for (index, coefficient) in self.echo.fir.iter().enumerate() {
                row[195 + index] = i32::from(*coefficient);
            }
            row[203] = i32::from(self.clock.sample);
            trace.push_back(row);
            if trace.len() > 600_000 {
                trace.pop_front();
            }
        }
    }

    fn echo28(&mut self) {
        self.echo._readonly = self.echo.readonly;
    }

    fn echo29(&mut self, ram: &mut [u8]) {
        self.echo._page = self.echo.page;
        if self.echo._offset == 0 {
            self.echo._length = u16::from(self.echo.delay) << 11;
        }
        self.echo._offset = self.echo._offset.wrapping_add(4);
        if self.echo._offset >= self.echo._length {
            self.echo._offset = 0;
        }
        self.echo_write(ram, 0);
        self.echo._readonly = self.echo.readonly;
    }

    fn echo30(&mut self, ram: &mut [u8]) {
        self.echo_write(ram, 1);
    }

    fn misc27(&mut self) {
        for voice in &mut self.voice {
            voice._modulate = voice.modulate;
        }
    }

    fn misc28(&mut self) {
        for voice in &mut self.voice {
            voice._noise = voice.noise;
            voice._echo = voice.echo;
        }
        self.brr._bank = self.brr.bank;
    }

    fn misc29(&mut self) {
        self.clock.sample = !self.clock.sample;
        if self.clock.sample {
            for voice in &mut self.voice {
                voice._keylatch &= !voice._keyon;
            }
        }
    }

    fn misc30(&mut self) {
        if self.clock.sample {
            for voice in &mut self.voice {
                voice._keyon = voice._keylatch;
                voice._keyoff = voice.keyoff;
            }
        }
        self.counter_tick();
        if self.counter_poll(u32::from(self.noise.frequency)) {
            let feedback = (self.noise.lfsr << 13) ^ (self.noise.lfsr << 14);
            self.noise.lfsr = (feedback & 0x4000) | (self.noise.lfsr >> 1);
        }
    }
}

fn clamp16(value: i32) -> i32 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn setup() -> (Dsp, Vec<u8>) {
        let mut ram = vec![0u8; 0x1_0000];
        // Sample directory at $0200: start/loop at $0300.
        ram[0x0200] = 0x00;
        ram[0x0201] = 0x03;
        ram[0x0202] = 0x00;
        ram[0x0203] = 0x03;
        // One non-looping BRR block, range 0/filter 0, end bit set.
        ram[0x0300] = 0x03;
        for (index, byte) in [0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]
            .into_iter()
            .enumerate()
        {
            ram[0x0301 + index] = byte;
        }

        let mut dsp = Dsp::new();
        dsp.write(0x0C, 0x7F); // MVOLL
        dsp.write(0x1C, 0x7F); // MVOLR
        dsp.write(0x00, 0x7F); // V0VOLL
        dsp.write(0x01, 0x7F); // V0VOLR
        dsp.write(0x02, 0x00); // pitch low
        dsp.write(0x03, 0x10); // pitch high -> 0x1000
        dsp.write(0x04, 0x00); // source 0
        dsp.write(0x05, 0x00); // ADSR0
        dsp.write(0x06, 0x00); // ADSR1
        dsp.write(0x07, 0x7F); // GAIN direct, max
        dsp.write(0x5D, 0x02); // DIR -> $0200
        dsp.write(0x6C, 0x00); // unmute, no reset
        dsp.write(0x4C, 0x01); // KON voice 0
        (dsp, ram)
    }

    #[test]
    fn register_file_round_trips() {
        let mut dsp = Dsp::new();
        dsp.write(0x12, 0x34);
        assert_eq!(dsp.read(0x12), 0x34);
        assert_eq!(dsp.read(0x92), 0x34);
        // `$80-$FF` read as mirrors of `$00-$7F`, but writes there are dropped.
        dsp.write(0x92, 0x56);
        assert_eq!(dsp.read(0x12), 0x34);
        assert_eq!(dsp.read(0x92), 0x34);
    }

    #[test]
    fn kon_produces_audio_and_end_flag() {
        let (mut dsp, mut ram) = setup();
        let mut nonzero = false;
        for _ in 0..64 {
            let (left, right) = dsp.run_sample(&mut ram);
            nonzero |= left != 0 || right != 0;
        }
        assert!(nonzero, "DSP produced only silence");
        assert_ne!(dsp.read(0x7C) & 0x01, 0, "ENDX voice 0 not set");
    }

    #[test]
    fn mute_silences_output() {
        let (mut dsp, mut ram) = setup();
        dsp.write(0x6C, 0x40); // mute
        for _ in 0..32 {
            assert_eq!(dsp.run_sample(&mut ram), (0, 0));
        }
    }

    #[test]
    fn reset_preserves_the_register_file() {
        // ares `DSP::power(reset = true)` clears the decoded voice/echo/noise
        // state but does not refill or zero `registers[]`, so a soft reset
        // keeps the power-on random pattern (and any driver-written value).
        let (mut dsp, _ram) = setup();
        dsp.write(0x4C, 0xA5);
        dsp.reset();
        assert_eq!(dsp.read(0x4C), 0xA5);
    }
}
