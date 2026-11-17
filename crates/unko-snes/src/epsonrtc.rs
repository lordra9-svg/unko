//! Epson RTC-4513 real-time clock.
//!
//! Ported from the ares `EpsonRTC` component. The chip sits on `$F9`
//! (`SPC7110-RAM-EPSONRTC`) boards at `$00-$3F,$80-$BF:$4840-$4842` and keeps
//! a battery-backed BCD calendar.
//!
//! The register interface is a three-port serial protocol: `$4840` selects the
//! chip, `$4841` carries the nibble/data stream and `$4842` reports readiness.
//! ares emulates the RTC-4513's *invalid* BCD behaviour bit-for-bit, so the
//! tick functions below keep the same odd increments.

/// The 2,097,152 Hz oscillator (`32768 * 64`), i.e. one tick per 1/2^21 s.
pub const RTC_CLOCK: u32 = 32_768 * 64;

/// Days per month, indexed by the packed BCD month (ares table, including its
/// index-0 entry).
const DAYS_IN_MONTH: [u32; 32] = [
    30, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31, 30, 31, 30, 31, 30, 31, 30, 31, 30, 31, 30,
    31, 30, 31, 30, 31, 30, 31, 30,
];

/// Serial protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RtcState {
    /// Waiting for the `$03` (write) or `$0c` (read) command.
    #[default]
    Mode,
    /// Command accepted; the next nibble is the register offset.
    Seek,
    /// Streaming register reads.
    Read,
    /// Streaming register writes.
    Write,
}

/// The Epson RTC-4513.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EpsonRtc {
    /// Oscillator ticks within the current second (a 21-bit counter).
    pub clocks: u32,
    /// Whole seconds since power-on.
    pub seconds: u32,

    chipselect: u8,
    state: RtcState,
    mdr: u8,
    offset: u8,
    /// Ticks left before the port becomes ready again.
    wait: u32,
    ready: bool,
    holdtick: bool,

    secondlo: u8,
    secondhi: u8,
    batteryfailure: u8,

    minutelo: u8,
    minutehi: u8,
    resync: u8,

    hourlo: u8,
    hourhi: u8,
    meridian: u8,

    daylo: u8,
    dayhi: u8,
    dayram: u8,

    monthlo: u8,
    monthhi: u8,
    monthram: u8,

    yearlo: u8,
    yearhi: u8,

    weekday: u8,

    hold: u8,
    calendar: u8,
    irqflag: u8,
    roundseconds: u8,

    irqmask: u8,
    irqduty: u8,
    irqperiod: u8,

    pause: u8,
    stop: u8,
    /// Astronomical (24-hour) time instead of 12-hour + meridian.
    atime: u8,
    test: u8,
}

impl Default for EpsonRtc {
    fn default() -> Self {
        Self {
            clocks: 0,
            seconds: 0,
            chipselect: 0,
            state: RtcState::Mode,
            mdr: 0,
            offset: 0,
            wait: 0,
            ready: false,
            holdtick: false,
            secondlo: 0,
            secondhi: 0,
            batteryfailure: 1,
            minutelo: 0,
            minutehi: 0,
            resync: 0,
            hourlo: 0,
            hourhi: 0,
            meridian: 0,
            daylo: 0,
            dayhi: 0,
            dayram: 0,
            monthlo: 0,
            monthhi: 0,
            monthram: 0,
            yearlo: 0,
            yearhi: 0,
            weekday: 0,
            hold: 0,
            calendar: 0,
            irqflag: 0,
            roundseconds: 0,
            irqmask: 0,
            irqduty: 0,
            irqperiod: 0,
            pause: 0,
            stop: 0,
            atime: 0,
            test: 0,
        }
    }
}

impl EpsonRtc {
    /// A powered-on chip (ares `EpsonRTC::initialize` + `power`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the calendar registers and the serial protocol.
    pub fn power(&mut self) {
        *self = Self::default();
    }

    /// Load the calendar from a Unix timestamp (ares `synchronize`).
    ///
    /// Hosts call this once from their own clock; the emulation itself never
    /// does, so runs stay deterministic.
    pub fn synchronize(&mut self, unix: i64) {
        let secs = unix.rem_euclid(86_400);
        let days = unix.div_euclid(86_400);
        let (hour, minute, second) = (
            (secs / 3600) as u32,
            ((secs % 3600) / 60) as u32,
            (secs % 60) as u32,
        );
        self.secondlo = (second % 10) as u8;
        self.secondhi = (second / 10) as u8;
        self.minutelo = (minute % 10) as u8;
        self.minutehi = (minute / 10) as u8;
        if self.atime != 0 {
            self.hourlo = (hour % 10) as u8;
            self.hourhi = (hour / 10) as u8;
        } else {
            self.meridian = u8::from(hour >= 12);
            let mut hour = hour % 12;
            if hour == 0 {
                hour = 12;
            }
            self.hourlo = (hour % 10) as u8;
            self.hourhi = (hour / 10) as u8;
        }
        // Civil-from-days (Howard Hinnant's algorithm), so the calendar does
        // not depend on the host's timezone database.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = if month <= 2 { y + 1 } else { y };

        self.daylo = (day % 10) as u8;
        self.dayhi = (day / 10) as u8;
        self.monthlo = (month % 10) as u8;
        self.monthhi = (month / 10) as u8;
        let year = year.rem_euclid(100) as u32;
        self.yearlo = (year % 10) as u8;
        self.yearhi = (year / 10) as u8;
        self.weekday = days.rem_euclid(7) as u8; // 1970-01-01 was a Thursday (4)
        self.weekday = (self.weekday + 4) % 7;
        self.resync = 1;
    }

    /// Read one of `$4840-$4842` (ares `EpsonRTC::read`).
    pub fn read(&mut self, address: usize, data: u8) -> u8 {
        match address & 3 {
            0 => self.chipselect,
            1 => {
                if self.chipselect != 1 || !self.ready {
                    return 0;
                }
                if self.state == RtcState::Write {
                    return self.mdr;
                }
                if self.state != RtcState::Read {
                    return 0;
                }
                self.ready = false;
                self.wait = 8;
                let value = self.rtc_read(self.offset);
                self.offset = (self.offset + 1) & 0x0F;
                value
            }
            2 => u8::from(self.ready) << 7,
            _ => data,
        }
    }

    /// Write one of `$4840-$4842` (ares `EpsonRTC::write`).
    pub fn write(&mut self, address: usize, data: u8) {
        let data = data & 0x0F;
        match address & 3 {
            0 => {
                self.chipselect = data & 0x03;
                if self.chipselect != 1 {
                    self.rtc_reset();
                }
                self.ready = true;
            }
            1 => {
                if self.chipselect != 1 || !self.ready {
                    return;
                }
                match self.state {
                    RtcState::Mode => {
                        if data != 0x03 && data != 0x0C {
                            return;
                        }
                        self.state = RtcState::Seek;
                        self.ready = false;
                        self.wait = 8;
                        self.mdr = data;
                    }
                    RtcState::Seek => {
                        if self.mdr == 0x03 {
                            self.state = RtcState::Write;
                        }
                        if self.mdr == 0x0C {
                            self.state = RtcState::Read;
                        }
                        self.offset = data;
                        self.ready = false;
                        self.wait = 8;
                        self.mdr = data;
                    }
                    RtcState::Write => {
                        self.rtc_write(self.offset, data);
                        self.offset = (self.offset + 1) & 0x0F;
                        self.ready = false;
                        self.wait = 8;
                        self.mdr = data;
                    }
                    RtcState::Read => {}
                }
            }
            _ => {}
        }
    }

    /// Advance one oscillator tick (ares `EpsonRTC::main`).
    pub fn tick(&mut self) {
        if self.wait != 0 {
            self.wait -= 1;
            if self.wait == 0 {
                self.ready = true;
            }
        }

        self.clocks = (self.clocks + 1) & 0x1F_FFFF;
        // ares compares the masked counter against zero, so these fire for the
        // first 256/16384/32768 ticks of every second.
        if self.clocks < 0x100 {
            self.round_seconds();
        }
        if self.clocks < 0x4000 {
            self.duty();
        }
        if self.clocks < 0x8000 {
            self.irq(0);
        }
        if self.clocks == 0 {
            self.seconds = self.seconds.wrapping_add(1);
            self.irq(1);
            if self.seconds.is_multiple_of(60) {
                self.irq(2);
            }
            if self.seconds.is_multiple_of(1440) {
                self.irq(3);
                self.seconds = 0;
            }
            self.tick_second_wrap();
        }
    }

    /// ares `rtcReset`.
    fn rtc_reset(&mut self) {
        self.state = RtcState::Mode;
        self.offset = 0;
        self.resync = 0;
        self.pause = 0;
        self.test = 0;
    }

    /// ares `rtcRead`. Reading register 13 clears the IRQ flag.
    fn rtc_read(&mut self, address: u8) -> u8 {
        match address & 0x0F {
            0 => self.secondlo,
            1 => self.secondhi | (self.batteryfailure << 3),
            2 => self.minutelo,
            3 => self.minutehi | (self.resync << 3),
            4 => self.hourlo,
            5 => self.hourhi | (self.meridian << 2) | (self.resync << 3),
            6 => self.daylo,
            7 => self.dayhi | (self.dayram << 2) | (self.resync << 3),
            8 => self.monthlo,
            9 => self.monthhi | (self.monthram << 1) | (self.resync << 3),
            10 => self.yearlo,
            11 => self.yearhi,
            12 => self.weekday | (self.resync << 3),
            13 => {
                let readflag = self.irqflag & (1 - self.irqmask);
                self.irqflag = 0;
                (self.hold | (self.calendar << 1) | (readflag << 2) | (self.roundseconds << 3))
                    & 0x0F
            }
            14 => (self.irqmask | (self.irqduty << 1) | (self.irqperiod << 2)) & 0x0F,
            _ => (self.pause | (self.stop << 1) | (self.atime << 2) | (self.test << 3)) & 0x0F,
        }
    }

    /// ares `rtcWrite`. Reading `$4841` clears `irqflag`, so this needs `&mut`.
    fn rtc_write(&mut self, address: u8, data: u8) {
        let data = data & 0x0F;
        match address & 0x0F {
            0 => self.secondlo = data,
            1 => {
                self.secondhi = data & 0x07;
                self.batteryfailure = data >> 3;
            }
            2 => self.minutelo = data,
            3 => self.minutehi = data & 0x07,
            4 => self.hourlo = data,
            5 => {
                self.hourhi = data & 0x03;
                self.meridian = (data >> 2) & 1;
                if self.atime == 1 {
                    self.meridian = 0;
                }
                if self.atime == 0 {
                    self.hourhi &= 1;
                }
            }
            6 => self.daylo = data,
            7 => {
                self.dayhi = data & 0x03;
                self.dayram = (data >> 2) & 1;
            }
            8 => self.monthlo = data,
            9 => {
                self.monthhi = data & 1;
                self.monthram = (data >> 1) & 3;
            }
            10 => self.yearlo = data,
            11 => self.yearhi = data,
            12 => self.weekday = data & 0x07,
            13 => {
                let held = self.hold;
                self.hold = data & 1;
                self.calendar = (data >> 1) & 1;
                self.roundseconds = (data >> 3) & 1;
                if held == 1 && self.hold == 0 && self.holdtick {
                    // A second ticked while held: advance one on resume.
                    self.holdtick = false;
                    self.tick_second();
                }
            }
            14 => {
                self.irqmask = data & 1;
                self.irqduty = (data >> 1) & 1;
                self.irqperiod = (data >> 2) & 3;
            }
            _ => {
                self.pause = data & 1;
                self.stop = (data >> 1) & 1;
                self.atime = (data >> 2) & 1;
                self.test = (data >> 3) & 1;
                if self.atime == 1 {
                    self.meridian = 0;
                }
                if self.atime == 0 {
                    self.hourhi &= 1;
                }
                if self.pause != 0 {
                    self.secondlo = 0;
                    self.secondhi = 0;
                }
            }
        }
    }

    fn irq(&mut self, period: u8) {
        if self.stop != 0 || self.pause != 0 {
            return;
        }
        if period == self.irqperiod {
            self.irqflag = 1;
        }
    }

    fn duty(&mut self) {
        if self.irqduty != 0 {
            self.irqflag = 0;
        }
    }

    fn round_seconds(&mut self) {
        if self.roundseconds == 0 {
            return;
        }
        self.roundseconds = 0;
        if self.secondhi >= 3 {
            self.tick_minute();
        }
        self.secondlo = 0;
        self.secondhi = 0;
    }

    /// ares `tick` (one whole second).
    fn tick_second_wrap(&mut self) {
        if self.stop != 0 || self.pause != 0 {
            return;
        }
        if self.hold != 0 {
            self.holdtick = true;
            return;
        }
        self.resync = 1;
        self.tick_second();
    }

    // Bit-perfect emulation of the RTC-4513's invalid BCD values (ares).

    fn tick_second(&mut self) {
        if self.secondlo <= 8 || self.secondlo == 12 {
            self.secondlo += 1;
        } else {
            self.secondlo = 0;
            if self.secondhi < 5 {
                self.secondhi += 1;
            } else {
                self.secondhi = 0;
                self.tick_minute();
            }
        }
    }

    fn tick_minute(&mut self) {
        if self.minutelo <= 8 || self.minutelo == 12 {
            self.minutelo += 1;
        } else {
            self.minutelo = 0;
            if self.minutehi < 5 {
                self.minutehi += 1;
            } else {
                self.minutehi = 0;
                self.tick_hour();
            }
        }
    }

    fn tick_hour(&mut self) {
        if self.atime != 0 {
            if self.hourhi < 2 {
                if self.hourlo <= 8 || self.hourlo == 12 {
                    self.hourlo += 1;
                } else {
                    self.hourlo = u8::from(self.hourlo & 1 == 0);
                    self.hourhi += 1;
                }
            } else if self.hourlo != 3 && self.hourlo & 4 == 0 {
                if self.hourlo <= 8 || self.hourlo >= 12 {
                    self.hourlo += 1;
                } else {
                    self.hourlo = u8::from(self.hourlo & 1 == 0);
                    self.hourhi += 1;
                }
            } else {
                self.hourlo = u8::from(self.hourlo & 1 == 0);
                self.hourhi = 0;
                self.tick_day();
            }
        } else if self.hourhi == 0 {
            if self.hourlo <= 8 || self.hourlo == 12 {
                self.hourlo += 1;
            } else {
                self.hourlo = u8::from(self.hourlo & 1 == 0);
                self.hourhi ^= 1;
            }
        } else {
            if self.hourlo & 1 != 0 {
                self.meridian ^= 1;
            }
            if self.hourlo < 2
                || self.hourlo == 4
                || self.hourlo == 5
                || self.hourlo == 8
                || self.hourlo == 12
            {
                self.hourlo += 1;
            } else {
                self.hourlo = u8::from(self.hourlo & 1 == 0);
                self.hourhi ^= 1;
            }
            if self.meridian == 0 && self.hourlo & 1 == 0 {
                self.tick_day();
            }
        }
    }

    fn tick_day(&mut self) {
        if self.calendar == 0 {
            return;
        }
        self.weekday = (self.weekday + 1) + u8::from(self.weekday == 6);
        self.weekday &= 0x07;

        let mut days = DAYS_IN_MONTH[((self.monthhi << 4) | self.monthlo) as usize & 31];
        if days == 28 {
            // Add a day for leap years.
            if (self.yearhi & 1) == 0 && self.yearlo & 3 == 0 {
                days += 1;
            }
            if (self.yearhi & 1) == 1 && self.yearlo.wrapping_sub(2) & 3 == 0 {
                days += 1;
            }
        }

        if days == 28 && (self.dayhi == 3 || (self.dayhi == 2 && self.daylo >= 8)) {
            self.daylo = 1;
            self.dayhi = 0;
            self.tick_month();
            return;
        }
        if days == 29
            && (self.dayhi == 3 || (self.dayhi == 2 && self.daylo > 8 && self.daylo != 12))
        {
            self.daylo = 1;
            self.dayhi = 0;
            self.tick_month();
            return;
        }
        if days == 30
            && (self.dayhi == 3 || (self.dayhi == 2 && (self.daylo == 10 || self.daylo == 14)))
        {
            self.daylo = 1;
            self.dayhi = 0;
            self.tick_month();
            return;
        }
        if days == 31 && self.dayhi == 3 && self.daylo & 3 != 0 {
            self.daylo = 1;
            self.dayhi = 0;
            self.tick_month();
            return;
        }

        if self.daylo <= 8 || self.daylo == 12 {
            self.daylo += 1;
        } else {
            self.daylo = u8::from(self.daylo & 1 == 0);
            self.dayhi += 1;
        }
    }

    fn tick_month(&mut self) {
        if self.monthhi == 0 || self.monthlo & 2 == 0 {
            if self.monthlo <= 8 || self.monthlo == 12 {
                self.monthlo += 1;
            } else {
                self.monthlo = u8::from(self.monthlo & 1 == 0);
                self.monthhi ^= 1;
            }
        } else {
            self.monthlo = u8::from(self.monthlo & 1 == 0);
            self.monthhi = 0;
            self.tick_year();
        }
    }

    fn tick_year(&mut self) {
        if self.yearlo <= 8 || self.yearlo == 12 {
            self.yearlo += 1;
        } else {
            self.yearlo = u8::from(self.yearlo & 1 == 0);
            if self.yearhi <= 8 || self.yearhi == 12 {
                self.yearhi += 1;
            } else {
                self.yearhi = u8::from(self.yearhi & 1 == 0);
            }
        }
    }

    /// The packed BCD calendar as `(year, month, day, hour, minute, second)`
    /// for tests and debug output.
    pub fn calendar(&self) -> (u8, u8, u8, u8, u8, u8) {
        let bcd = |hi: u8, lo: u8| hi * 10 + lo;
        (
            bcd(self.yearhi, self.yearlo),
            bcd(self.monthhi, self.monthlo),
            bcd(self.dayhi, self.daylo),
            bcd(self.hourhi, self.hourlo),
            bcd(self.minutehi, self.minutelo),
            bcd(self.secondhi, self.secondlo),
        )
    }

    /// Current `$4840` chip-select value (debug).
    pub const fn chipselect(&self) -> u8 {
        self.chipselect
    }

    /// Expose the IRQ flag for tests.
    pub const fn irq_flag(&self) -> u8 {
        self.irqflag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Select the chip and start a register stream at `offset`.
    fn open(rtc: &mut EpsonRtc, command: u8, offset: u8) {
        // Deselecting first resets the serial protocol back to Mode.
        rtc.write(0, 0x00);
        rtc.write(0, 0x01);
        rtc.write(1, command);
        for _ in 0..8 {
            rtc.tick();
        }
        rtc.write(1, offset);
        for _ in 0..8 {
            rtc.tick();
        }
    }

    fn read_register(rtc: &mut EpsonRtc) -> u8 {
        let value = rtc.read(1, 0);
        for _ in 0..8 {
            rtc.tick();
        }
        value
    }

    fn write_register(rtc: &mut EpsonRtc, value: u8) {
        rtc.write(1, value);
        for _ in 0..8 {
            rtc.tick();
        }
    }

    #[test]
    fn powers_up_with_a_battery_failure() {
        let mut rtc = EpsonRtc::new();
        assert_eq!(rtc.calendar(), (0, 0, 0, 0, 0, 0));
        open(&mut rtc, 0x0C, 0);
        // Register 1 bit 3 reports the battery failure.
        let _ = read_register(&mut rtc);
        assert_eq!(read_register(&mut rtc) & 0x08, 0x08);
    }

    #[test]
    fn chip_select_is_visible_on_port_zero() {
        let mut rtc = EpsonRtc::new();
        assert_eq!(rtc.read(0, 0), 0);
        rtc.write(0, 0x01);
        assert_eq!(rtc.read(0, 0), 1);
        // Port 2 bit 7 reports readiness.
        assert_eq!(rtc.read(2, 0) & 0x80, 0x80);
        rtc.write(2, 0xFF);
        // Port 3 is open bus.
        assert_eq!(rtc.read(3, 0x5A), 0x5A);
    }

    #[test]
    fn serial_read_stream() {
        let mut rtc = EpsonRtc::new();
        rtc.synchronize(946_684_800); // 2000-01-01T00:00:00Z
        open(&mut rtc, 0x0C, 0);
        assert_eq!(read_register(&mut rtc), 0x00); // second low
        assert_eq!(read_register(&mut rtc) & 0x07, 0x00); // second high
        assert_eq!(read_register(&mut rtc), 0x00); // minute low
        assert_eq!(read_register(&mut rtc) & 0x07, 0x00); // minute high
        assert_eq!(read_register(&mut rtc), 0x02); // hour low: 12
        assert_eq!(read_register(&mut rtc) & 0x03, 0x01); // hour high: 1
        assert_eq!(read_register(&mut rtc), 0x01); // day low
        assert_eq!(read_register(&mut rtc) & 0x03, 0x00); // day high
        assert_eq!(read_register(&mut rtc), 0x01); // month low
        assert_eq!(read_register(&mut rtc) & 0x01, 0x00); // month high
        assert_eq!(read_register(&mut rtc), 0x00); // year low
        assert_eq!(read_register(&mut rtc), 0x00); // year high
        assert_eq!(read_register(&mut rtc) & 0x07, 0x06); // weekday: Saturday
    }

    #[test]
    fn serial_write_stream_round_trips() {
        let values = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x01, 0x09, 0x02, 0x03, 0x04, 0x05,
        ];
        let mut rtc = EpsonRtc::new();
        open(&mut rtc, 0x03, 0);
        for value in values {
            write_register(&mut rtc, value);
        }
        // Footnotes in registers 1/3/5/7/9 are clear, so the nibbles read back.
        open(&mut rtc, 0x0C, 0);
        for (register, expected) in values.iter().enumerate() {
            assert_eq!(
                read_register(&mut rtc),
                *expected,
                "register {register} did not round-trip"
            );
        }
    }

    #[test]
    fn ticks_advance_the_calendar() {
        let mut rtc = EpsonRtc::new();
        rtc.synchronize(946_684_800); // 2000-01-01T00:00:00Z
        for _ in 0..RTC_CLOCK {
            rtc.tick();
        }
        assert_eq!(rtc.calendar(), (0, 1, 1, 12, 0, 1));
    }

    #[test]
    fn leap_day_is_handled() {
        // 2000-02-28T23:59:59Z in 24-hour mode.
        let mut rtc = EpsonRtc::new();
        rtc.atime = 1;
        rtc.synchronize(951_782_399);
        assert_eq!(rtc.calendar(), (0, 2, 28, 23, 59, 59));
        rtc.calendar = 1;
        rtc.tick_second_wrap();
        assert_eq!(rtc.calendar(), (0, 2, 29, 0, 0, 0));
        // A full day later is 1 March.
        for _ in 0..86_400 {
            rtc.tick_second_wrap();
        }
        assert_eq!(rtc.calendar(), (0, 3, 1, 0, 0, 0));
    }

    #[test]
    fn irq_flag_follows_the_period_and_mask() {
        let mut rtc = EpsonRtc::new();
        // irqperiod 0: the flag is set during the first half second.
        rtc.irqperiod = 0;
        rtc.tick();
        assert_eq!(rtc.irq_flag(), 1);

        // Reading register 13 reports the flag in bit 2 and clears it.
        rtc.irqmask = 0;
        assert_eq!(rtc.rtc_read(13) & 0x04, 0x04);
        assert_eq!(rtc.irq_flag(), 0);

        // With irqmask set the reported bit stays clear.
        rtc.irqflag = 1;
        rtc.irqmask = 1;
        assert_eq!(rtc.rtc_read(13) & 0x04, 0x00);
        assert_eq!(rtc.irq_flag(), 0);

        // A period that never matches leaves the flag alone.
        let mut quiet = EpsonRtc::new();
        quiet.irqperiod = 3;
        for _ in 0..0x8000 {
            quiet.tick();
        }
        assert_eq!(quiet.irq_flag(), 0);
    }

    #[test]
    fn stop_halts_the_clock() {
        let mut rtc = EpsonRtc::new();
        rtc.synchronize(946_684_800);
        rtc.stop = 1;
        for _ in 0..RTC_CLOCK {
            rtc.tick();
        }
        assert_eq!(rtc.calendar(), (0, 1, 1, 12, 0, 0));
        rtc.stop = 0;
        for _ in 0..RTC_CLOCK {
            rtc.tick();
        }
        assert_eq!(rtc.calendar(), (0, 1, 1, 12, 0, 1));
    }

    #[test]
    fn hold_defers_a_pending_second() {
        let mut rtc = EpsonRtc::new();
        rtc.synchronize(946_684_800);
        rtc.hold = 1;
        for _ in 0..RTC_CLOCK {
            rtc.tick();
        }
        assert!(rtc.holdtick);
        assert_eq!(rtc.calendar(), (0, 1, 1, 12, 0, 0));
        // Releasing the hold applies the deferred second.
        open(&mut rtc, 0x03, 13);
        write_register(&mut rtc, 0x00);
        assert_eq!(rtc.calendar(), (0, 1, 1, 12, 0, 1));
    }
}
