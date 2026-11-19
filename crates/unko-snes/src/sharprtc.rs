//! Sharp S-RTC (real-time clock) coprocessor.
//!
//! Used by `EXHIROM-RAM-SHARPRTC` boards (cartridge-type high nibble `$5`) and
//! mapped at `$00-$3F/$80-$BF:$2800-$2801`. The four-bit register protocol and
//! the calendar arithmetic follow the ISC-licensed ares
//! `sfc/coprocessor/sharprtc` source: even addresses read the next nibble of a
//! 13-nibble record after a `$0D` command, odd addresses take one of the
//! `$0D`/`$0E`/`$0F` commands or a data nibble.
//!
//! The core has no host clock, so the time starts at a documented default
//! (2000-01-01 00:00:00 UTC) and the host can set it with
//! [`crate::Snes::set_sharp_rtc_time`]. It then ticks once per emulated second.

const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Seconds from the Unix epoch to the default RTC time, 2000-01-01 00:00:00.
pub const DEFAULT_RTC_UNIX: i64 = 946_684_800;

/// Sharp RTC protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum State {
    Ready,
    Command,
    Read,
    Write,
}

/// Sharp S-RTC register file and calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SharpRtc {
    state: State,
    index: i32,
    second: u32,
    minute: u32,
    hour: u32,
    day: u32,
    month: u32,
    /// Years since 1000 (ares `year`, e.g. 2026 = 1126).
    year: u32,
    /// 0 = Sunday.
    weekday: u32,
}

impl Default for SharpRtc {
    fn default() -> Self {
        Self::new()
    }
}

impl SharpRtc {
    /// Create the RTC at the default time with a ready protocol state.
    pub fn new() -> Self {
        let mut rtc = Self {
            state: State::Ready,
            index: -1,
            second: 0,
            minute: 0,
            hour: 0,
            day: 1,
            month: 1,
            year: 1000,
            weekday: 6, // 2000-01-01 was a Saturday
        };
        rtc.set_unix(DEFAULT_RTC_UNIX);
        rtc
    }

    /// ares `SharpRTC::power`: reset the protocol state, keep the calendar.
    pub fn power(&mut self) {
        self.state = State::Ready;
        self.index = -1;
    }

    /// Load the calendar from a Unix timestamp.
    pub fn set_unix(&mut self, unix: i64) {
        let days = unix.div_euclid(86_400);
        let seconds = unix.rem_euclid(86_400) as u32;
        self.second = seconds % 60;
        self.minute = (seconds / 60) % 60;
        self.hour = (seconds / 3600) % 24;
        // 1970-01-01 was a Thursday (4).
        self.weekday = (days + 4).rem_euclid(7) as u32;
        let (year, month, day) = civil_from_days(days);
        // ares stores years since 1000 (`year = 900 + tm_year`).
        self.year = (year as u32).saturating_sub(1000);
        self.month = month;
        self.day = day;
    }

    /// The current calendar: `(second, minute, hour, day, month, year, weekday)`
    /// with `year` as an absolute year (e.g. 2026) and `weekday` 0 = Sunday.
    pub const fn calendar(&self) -> (u32, u32, u32, u32, u32, u32, u32) {
        (
            self.second,
            self.minute,
            self.hour,
            self.day,
            self.month,
            1000 + self.year,
            self.weekday,
        )
    }

    /// Advance the calendar by one second.
    pub fn tick(&mut self) {
        self.second += 1;
        if self.second < 60 {
            return;
        }
        self.second = 0;
        self.minute += 1;
        if self.minute < 60 {
            return;
        }
        self.minute = 0;
        self.hour += 1;
        if self.hour < 24 {
            return;
        }
        self.hour = 0;
        self.tick_day();
    }

    fn tick_day(&mut self) {
        let mut days = DAYS_IN_MONTH[((self.month.max(1) - 1) % 12) as usize];
        if self.month == 2 && is_leap_year(1000 + self.year) {
            days += 1;
        }
        self.day += 1;
        if self.day <= days {
            return;
        }
        self.day = 1;
        self.tick_month();
    }

    fn tick_month(&mut self) {
        self.month += 1;
        if self.month <= 12 {
            return;
        }
        self.month = 1;
        self.year = (self.year + 1) & 0x0FFF;
    }

    fn read_nibble(&self, address: u32) -> u8 {
        match address {
            0 => (self.second % 10) as u8,
            1 => (self.second / 10) as u8,
            2 => (self.minute % 10) as u8,
            3 => (self.minute / 10) as u8,
            4 => (self.hour % 10) as u8,
            5 => (self.hour / 10) as u8,
            6 => (self.day % 10) as u8,
            7 => (self.day / 10) as u8,
            8 => self.month as u8,
            9 => (self.year % 10) as u8,
            10 => (self.year / 10 % 10) as u8,
            11 => (self.year / 100) as u8,
            12 => self.weekday as u8,
            _ => 0,
        }
    }

    fn write_nibble(&mut self, address: u32, data: u8) {
        let data = u32::from(data & 0x0F);
        match address {
            0 => self.second = self.second / 10 * 10 + data,
            1 => self.second = data * 10 + self.second % 10,
            2 => self.minute = self.minute / 10 * 10 + data,
            3 => self.minute = data * 10 + self.minute % 10,
            4 => self.hour = self.hour / 10 * 10 + data,
            5 => self.hour = data * 10 + self.hour % 10,
            6 => self.day = self.day / 10 * 10 + data,
            7 => self.day = data * 10 + self.day % 10,
            8 => self.month = data,
            9 => self.year = self.year / 10 * 10 + data,
            10 => self.year = self.year / 100 * 100 + data * 10 + self.year % 10,
            11 => self.year = data * 100 + self.year % 100,
            12 => self.weekday = data,
            _ => {}
        }
    }

    /// Read `$2800` (even) or `$2801` (odd). `mdr` is the open-bus value.
    pub fn read(&mut self, address: u32, mdr: u8) -> u8 {
        if address & 1 != 0 {
            return mdr;
        }
        if self.state != State::Read {
            return 0;
        }
        if self.index < 0 {
            self.index += 1;
            return 15;
        }
        if self.index > 12 {
            self.index = -1;
            return 15;
        }
        let value = self.read_nibble(self.index as u32);
        self.index += 1;
        value
    }

    /// Write `$2800` (even, ignored) or `$2801` (odd) with the low nibble.
    pub fn write(&mut self, address: u32, data: u8) {
        if address & 1 == 0 {
            return;
        }
        let data = data & 0x0F;
        match data {
            0x0D => {
                self.state = State::Read;
                self.index = -1;
                return;
            }
            0x0E => {
                self.state = State::Command;
                return;
            }
            0x0F => return,
            _ => {}
        }

        match self.state {
            State::Command => match data {
                0 => {
                    self.state = State::Write;
                    self.index = 0;
                }
                4 => {
                    // Reset the calendar.
                    self.second = 0;
                    self.minute = 0;
                    self.hour = 0;
                    self.day = 0;
                    self.month = 0;
                    self.year = 0;
                    self.weekday = 0;
                    self.state = State::Ready;
                    self.index = -1;
                }
                _ => self.state = State::Ready,
            },
            State::Write if self.index >= 0 && self.index < 12 => {
                self.write_nibble(self.index as u32, data);
                self.index += 1;
                if self.index == 12 {
                    self.weekday = calculate_weekday(1000 + self.year, self.month, self.day);
                }
            }
            _ => {}
        }
    }

    /// Whether `address` is the RTC window (`$2800-$2801` in banks that see
    /// the low 32 KiB).
    pub const fn window(address: u32) -> bool {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = address & 0xFFFF;
        (bank <= 0x3F || (bank >= 0x80 && bank <= 0xBF)) && (offset == 0x2800 || offset == 0x2801)
    }
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// ares `SharpRTC::calculateWeekday`, with the Sharp epoch of 1000-01-01
/// (a Wednesday).
fn calculate_weekday(year: u32, month: u32, day: u32) -> u32 {
    let mut y = 1000;
    let mut m = 1;
    let mut sum = 0;
    let year = year.max(1000);
    let month = month.clamp(1, 12);
    let day = day.clamp(1, 31);

    while y < year {
        sum += 365 + u32::from(is_leap_year(y));
        y += 1;
    }
    while m < month {
        let mut days = DAYS_IN_MONTH[((m - 1) % 12) as usize];
        if days == 28 && is_leap_year(y) {
            days += 1;
        }
        sum += days;
        m += 1;
    }
    sum += day - 1;
    (sum + 3) % 7
}

/// Convert days since 1970-01-01 to a civil `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_record(rtc: &mut SharpRtc) -> [u8; 13] {
        rtc.write(1, 0x0D);
        assert_eq!(rtc.read(0, 0), 15);
        let mut record = [0u8; 13];
        for nibble in &mut record {
            *nibble = rtc.read(0, 0);
        }
        assert_eq!(rtc.read(0, 0), 15);
        record
    }

    #[test]
    fn default_time_is_year_2000() {
        let rtc = SharpRtc::new();
        assert_eq!(rtc.calendar(), (0, 0, 0, 1, 1, 2000, 6));
    }

    #[test]
    fn unix_time_loads_the_calendar_and_weekday() {
        let mut rtc = SharpRtc::new();
        rtc.set_unix(1_700_000_000); // 2023-11-14 22:13:20 UTC, a Tuesday
        assert_eq!(rtc.calendar(), (20, 13, 22, 14, 11, 2023, 2));
    }

    #[test]
    fn read_command_streams_the_bcd_record() {
        let mut rtc = SharpRtc::new();
        rtc.set_unix(946_775_145); // 2000-01-02 01:05:45
        let record = read_record(&mut rtc);
        assert_eq!(record, [5, 4, 5, 0, 1, 0, 2, 0, 1, 0, 0, 10, 0]);
    }

    #[test]
    fn write_command_sets_the_calendar_and_weekday() {
        let mut rtc = SharpRtc::new();
        rtc.write(1, 0x0E);
        rtc.write(1, 0x00);
        // 2026-11-19 12:34:56, a Thursday.
        for nibble in [6, 5, 4, 3, 2, 1, 9, 1, 11, 6, 2, 10] {
            rtc.write(1, nibble);
        }
        assert_eq!(rtc.calendar(), (56, 34, 12, 19, 11, 2026, 4));
    }

    #[test]
    fn command_four_resets_the_calendar() {
        let mut rtc = SharpRtc::new();
        rtc.write(1, 0x0E);
        rtc.write(1, 0x04);
        assert_eq!(rtc.calendar(), (0, 0, 0, 0, 0, 1000, 0));
    }

    #[test]
    fn ticks_roll_over_minutes_hours_and_days() {
        let mut rtc = SharpRtc::new();
        rtc.set_unix(946_775_145); // 2000-01-02 01:05:45
        for _ in 0..15 {
            rtc.tick();
        }
        assert_eq!(rtc.calendar(), (0, 6, 1, 2, 1, 2000, 0));
        for _ in 0..60 {
            rtc.tick();
        }
        assert_eq!(rtc.calendar(), (0, 7, 1, 2, 1, 2000, 0));
        // 2000 is a leap year: February 29 exists.
        rtc.set_unix(951_696_000); // 2000-02-28 00:00:00
        for _ in 0..(2 * 86_400) {
            rtc.tick();
        }
        // ares leaves the weekday register unchanged on ticks (only the
        // host-time load and a write command update it).
        assert_eq!(rtc.calendar(), (0, 0, 0, 1, 3, 2000, 1));
    }

    #[test]
    fn weekday_calculation_matches_known_dates() {
        assert_eq!(calculate_weekday(2000, 1, 1), 6);
        assert_eq!(calculate_weekday(2026, 11, 19), 4);
        assert_eq!(calculate_weekday(2024, 2, 29), 4);
    }

    #[test]
    fn odd_address_returns_open_bus_and_reads_without_command_are_zero() {
        let mut rtc = SharpRtc::new();
        assert_eq!(rtc.read(1, 0x5A), 0x5A);
        assert_eq!(rtc.read(0, 0), 0);
    }

    #[test]
    fn windows_match_the_board_map() {
        for address in [0x00_2800, 0x00_2801, 0x3F_2801, 0x80_2800, 0xBF_2801] {
            assert!(SharpRtc::window(address), "{address:06X}");
        }
        for address in [0x40_2800, 0xC0_2801, 0x00_2802, 0x00_27FF] {
            assert!(!SharpRtc::window(address), "{address:06X}");
        }
    }
}
