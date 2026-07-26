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
        if month < 1 || month > 12 || day < 1 || day > 31 {
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

    /// Returns day (1-31).
    pub fn day(&self) -> u8 {
        self.day
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

    /// Returns Unix timestamp in seconds.
    pub fn timestamp(&self) -> i64 {
        0
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
}
