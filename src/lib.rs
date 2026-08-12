#![no_std]
#![deny(missing_docs)]

//! # `rusty_time`
//!
//! A `#![no_std]` + `alloc` sovereign Date, Time, DateTime, ISO-8601 parser/formatter,
//! and timezone offset calculation engine for the **Rusty Mill** ecosystem.

extern crate alloc;

use alloc::format;
use alloc::string::String;

/// Sovereign Date representation (Year, Month, Day).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// Creates a new Date instance.
    pub fn from_ymd(year: i32, month: u8, day: u8) -> Result<Self, &'static str> {
        if !(1..=12).contains(&month) {
            return Err("Invalid date parameters");
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err("Invalid date parameters");
        }
        Ok(Self { year, month, day })
    }

    /// Returns year.
    pub fn year(&self) -> i32 {
        self.year
    }

    /// Returns month (1-12).
    pub fn month(&self) -> u8 {
        self.month
    }

    /// Returns day of month (1-31, bounded by the month and year).
    pub fn day(&self) -> u8 {
        self.day
    }

    /// Returns the number of days between the Unix epoch (1970-01-01) and this date.
    ///
    /// Uses Howard Hinnant's `days_from_civil` algorithm, which is valid for
    /// every proleptic-Gregorian date representable by `i32::MIN..=i32::MAX` years.
    fn days_since_epoch(&self) -> i64 {
        let y = if self.month <= 2 {
            self.year as i64 - 1
        } else {
            self.year as i64
        };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (self.month as i64 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

/// Returns true if `year` is a Gregorian leap year.
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Returns the number of days in `month` (1-12) of `year`.
fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Sovereign Time representation (Hour, Minute, Second, Nanosecond).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time {
    hour: u8,
    minute: u8,
    second: u8,
    nano: u32,
}

impl Time {
    /// Creates a new Time instance.
    pub fn from_hms_nano(hour: u8, minute: u8, second: u8, nano: u32) -> Result<Self, &'static str> {
        if hour > 23 || minute > 59 || second > 59 {
            return Err("Invalid time parameters");
        }
        Ok(Self { hour, minute, second, nano })
    }

    /// Returns hour (0-23).
    pub fn hour(&self) -> u8 {
        self.hour
    }

    /// Returns minute (0-59).
    pub fn minute(&self) -> u8 {
        self.minute
    }

    /// Returns second (0-59).
    pub fn second(&self) -> u8 {
        self.second
    }

    /// Returns the nanosecond component (0-999_999_999).
    pub fn nanosecond(&self) -> u32 {
        self.nano
    }
}

/// Sovereign DateTime representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    date: Date,
    time: Time,
    offset_secs: i32,
}

impl DateTime {
    /// Creates a new DateTime instance.
    pub fn new(date: Date, time: Time, offset_secs: i32) -> Self {
        Self { date, time, offset_secs }
    }

    /// Returns the date component.
    pub fn date(&self) -> Date {
        self.date
    }

    /// Returns the time-of-day component.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Returns the UTC offset in seconds (positive east of UTC).
    pub fn offset_secs(&self) -> i32 {
        self.offset_secs
    }

    /// Returns Unix timestamp in seconds.
    pub fn timestamp(&self) -> i64 {
        let days = self.date.days_since_epoch();
        let time_of_day = self.time.hour() as i64 * 3600
            + self.time.minute() as i64 * 60
            + self.time.second() as i64;
        days * 86_400 + time_of_day - self.offset_secs as i64
    }

    /// Formats as ISO-8601 string representation.
    pub fn to_iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.date.year(),
            self.date.month(),
            self.date.day(),
            self.time.hour(),
            self.time.minute(),
            self.time.second()
        )
    }

    /// Parses an RFC 3339 datetime string (e.g. `"2026-08-12T01:18:55Z"` or
    /// `"2026-08-12T01:18:55.123456789+02:00"`) into a [`DateTime`].
    pub fn parse(s: &str) -> Result<Self, &'static str> {
        let bytes = s.as_bytes();
        if bytes.len() < 20 {
            return Err("Input too short for an RFC 3339 datetime");
        }

        let year = parse_digits(s, 0, 4)? as i32;
        expect_byte(bytes, 4, b'-')?;
        let month = parse_digits(s, 5, 7)? as u8;
        expect_byte(bytes, 7, b'-')?;
        let day = parse_digits(s, 8, 10)? as u8;

        match bytes[10] {
            b'T' | b't' | b' ' => {}
            _ => return Err("Expected date/time separator"),
        }

        let hour = parse_digits(s, 11, 13)? as u8;
        expect_byte(bytes, 13, b':')?;
        let minute = parse_digits(s, 14, 16)? as u8;
        expect_byte(bytes, 16, b':')?;
        let second = parse_digits(s, 17, 19)? as u8;

        let mut idx = 19;
        let mut nano: u32 = 0;
        if idx < bytes.len() && bytes[idx] == b'.' {
            idx += 1;
            let start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }
            if idx == start {
                return Err("Empty fractional seconds");
            }
            let mut value: u32 = 0;
            for i in 0..9 {
                let digit = if start + i < idx {
                    bytes[start + i] - b'0'
                } else {
                    0
                };
                value = value * 10 + digit as u32;
            }
            nano = value;
        }

        if idx >= bytes.len() {
            return Err("Missing timezone offset");
        }
        let offset_secs: i32 = match bytes[idx] {
            b'Z' | b'z' => {
                idx += 1;
                0
            }
            sign @ (b'+' | b'-') => {
                idx += 1;
                if idx + 5 > bytes.len() {
                    return Err("Invalid timezone offset");
                }
                let offset_hour = parse_digits(s, idx, idx + 2)? as i32;
                expect_byte(bytes, idx + 2, b':')?;
                let offset_minute = parse_digits(s, idx + 3, idx + 5)? as i32;
                idx += 5;
                let magnitude = offset_hour * 3600 + offset_minute * 60;
                if sign == b'-' {
                    -magnitude
                } else {
                    magnitude
                }
            }
            _ => return Err("Invalid timezone designator"),
        };

        if idx != bytes.len() {
            return Err("Unexpected trailing characters");
        }

        let date = Date::from_ymd(year, month, day)?;
        let time = Time::from_hms_nano(hour, minute, second, nano)?;
        Ok(DateTime::new(date, time, offset_secs))
    }
}

impl core::str::FromStr for DateTime {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Parses the ASCII digits `s[start..end]` as a `u32`.
fn parse_digits(s: &str, start: usize, end: usize) -> Result<u32, &'static str> {
    let slice = s
        .get(start..end)
        .ok_or("Unexpected end of input while parsing digits")?;
    let mut value: u32 = 0;
    for byte in slice.bytes() {
        if !byte.is_ascii_digit() {
            return Err("Expected an ASCII digit");
        }
        value = value * 10 + (byte - b'0') as u32;
    }
    Ok(value)
}

/// Checks that `bytes[index] == expected`.
fn expect_byte(bytes: &[u8], index: usize, expected: u8) -> Result<(), &'static str> {
    if bytes.get(index) != Some(&expected) {
        return Err("Unexpected character in RFC 3339 datetime");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_creation() {
        let d = Date::from_ymd(2026, 7, 25).unwrap();
        assert_eq!(d.year(), 2026);
        assert_eq!(d.month(), 7);
        assert_eq!(d.day(), 25);
    }

    #[test]
    fn iso8601_formatting() {
        let d = Date::from_ymd(2026, 7, 25).unwrap();
        let t = Time::from_hms_nano(15, 30, 0, 0).unwrap();
        let dt = DateTime::new(d, t, 0);
        assert_eq!(dt.to_iso8601(), "2026-07-25T15:30:00Z");
    }

    #[test]
    fn rejects_invalid_day_for_month() {
        assert!(Date::from_ymd(2025, 2, 29).is_err());
        assert!(Date::from_ymd(2024, 2, 29).is_ok());
        assert!(Date::from_ymd(2026, 4, 31).is_err());
    }

    #[test]
    fn parse_epoch() {
        let dt = DateTime::parse("1970-01-01T00:00:00Z").unwrap();
        assert_eq!(dt.timestamp(), 0);
    }

    #[test]
    fn parse_known_timestamp() {
        let dt = DateTime::parse("2000-01-01T00:00:00Z").unwrap();
        assert_eq!(dt.timestamp(), 946_684_800);
    }

    #[test]
    fn parse_round_trips_through_iso8601() {
        let dt = DateTime::parse("2026-08-12T01:18:55Z").unwrap();
        assert_eq!(dt.to_iso8601(), "2026-08-12T01:18:55Z");
        assert_eq!(dt.date().year(), 2026);
        assert_eq!(dt.time().nanosecond(), 0);
    }

    #[test]
    fn parse_fractional_seconds_and_offset() {
        let dt = DateTime::parse("2026-08-12T03:18:55.5+02:00").unwrap();
        assert_eq!(dt.time().nanosecond(), 500_000_000);
        // 03:18:55+02:00 is the same instant as 01:18:55Z.
        assert_eq!(dt.timestamp(), DateTime::parse("2026-08-12T01:18:55Z").unwrap().timestamp());
    }

    #[test]
    fn parse_lowercase_designators() {
        assert!(DateTime::parse("2026-08-12t01:18:55z").is_ok());
    }

    #[test]
    fn parse_negative_offset() {
        let dt = DateTime::parse("2026-08-11T23:18:55-02:00").unwrap();
        assert_eq!(dt.timestamp(), DateTime::parse("2026-08-12T01:18:55Z").unwrap().timestamp());
    }

    #[test]
    fn parse_rejects_malformed_input() {
        assert!(DateTime::parse("not-a-date").is_err());
        assert!(DateTime::parse("2026-08-12T01:18:55").is_err());
        assert!(DateTime::parse("2026-13-12T01:18:55Z").is_err());
        assert!(DateTime::parse("2026-08-12X01:18:55Z").is_err());
        assert!(DateTime::parse("2026-08-12T01:18:55+0200").is_err());
    }

    #[test]
    fn from_str_matches_parse() {
        use core::str::FromStr;
        let a = DateTime::parse("2026-08-12T01:18:55Z").unwrap();
        let b = DateTime::from_str("2026-08-12T01:18:55Z").unwrap();
        assert_eq!(a, b);
    }
}
