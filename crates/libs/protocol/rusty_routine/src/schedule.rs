//! Five-field cron, in UTC: `minute hour day-of-month month day-of-week`.
//!
//! Each field is `*`, a number, a range `a-b`, a list `a,b,c`, or any of
//! those with a step `/n`. Day of week is `0`–`7` with both `0` and `7`
//! Sunday. As in cron, when both day fields are restricted a day matches
//! if *either* does. No names, no seconds, no `L`/`W`/`#`: the subset the
//! workspace needs, written so a wrong expression fails at parse time.

use crate::Error;

/// A parsed cron expression. Each field is a set of permitted values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    /// Whether the day-of-month field was `*`; see [`Schedule::day_matches`].
    any_day: bool,
    /// Whether the day-of-week field was `*`.
    any_weekday: bool,
}

/// How far [`Schedule::next_after`] looks before giving up: past this an
/// expression names a date that does not exist (`31` in a 30-day month,
/// `30 2`).
const HORIZON_DAYS: u64 = 366 * 5;

impl Schedule {
    /// Parse `minute hour day month weekday`.
    pub fn parse(expr: &str) -> Result<Self, Error> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(Error::Schedule(format!(
                "expected 5 fields, got {} in {expr:?}",
                fields.len()
            )));
        }
        let minutes = field(fields[0], 0, 59, "minute")?;
        let hours = field(fields[1], 0, 23, "hour")?;
        let days = field(fields[2], 1, 31, "day of month")?;
        let months = field(fields[3], 1, 12, "month")?;
        let mut weekdays = field(fields[4], 0, 7, "day of week")?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays |= 1;
        }
        Ok(Schedule {
            minutes,
            hours: hours as u32,
            days: days as u32,
            months: months as u16,
            weekdays: (weekdays & 0x7f) as u8,
            any_day: fields[2] == "*",
            any_weekday: fields[4] == "*",
        })
    }

    /// The first scheduled minute strictly after `t` (seconds since the
    /// epoch), or `None` when nothing matches within five years.
    pub fn next_after(&self, t: u64) -> Option<u64> {
        let mut minute = t / 60 + 1;
        let end = minute + HORIZON_DAYS * 24 * 60;
        while minute < end {
            let (y, m, d, weekday) = civil(minute / (24 * 60));
            if self.months & (1 << m) == 0 {
                minute = next_day(minute);
                continue;
            }
            if !self.day_matches(d, weekday) {
                minute = next_day(minute);
                continue;
            }
            let h = (minute / 60) % 24;
            if self.hours & (1 << h) == 0 {
                minute = (minute / 60 + 1) * 60;
                continue;
            }
            if self.minutes & (1 << (minute % 60)) == 0 {
                minute += 1;
                continue;
            }
            let _ = y;
            return Some(minute * 60);
        }
        None
    }

    fn day_matches(&self, day: u64, weekday: u64) -> bool {
        let by_day = self.days & (1 << day) != 0;
        let by_weekday = self.weekdays & (1 << weekday) != 0;
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (false, true) => by_day,
            (true, false) => by_weekday,
            (false, false) => by_day || by_weekday,
        }
    }
}

/// The first minute of the day after the one `minute` is in.
fn next_day(minute: u64) -> u64 {
    (minute / (24 * 60) + 1) * 24 * 60
}

/// Year, month, day and weekday (0 = Sunday) of a day count since
/// 1970-01-01, by Howard Hinnant's `civil_from_days`.
fn civil(days: u64) -> (u64, u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    // 1970-01-01 was a Thursday.
    let weekday = (days + 4) % 7;
    (y, m, d, weekday)
}

/// Parse one field into a bit set over `min..=max`.
fn field(text: &str, min: u64, max: u64, what: &str) -> Result<u64, Error> {
    let bad = |why: &str| Error::Schedule(format!("{what} {text:?}: {why}"));
    let mut set = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u64>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| bad("step must be a positive number"))?,
            ),
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (number(a, min, max, &bad)?, number(b, min, max, &bad)?)
        } else {
            let n = number(range, min, max, &bad)?;
            // `5/10` means every tenth from 5, as in Vixie cron.
            (n, if step > 1 { max } else { n })
        };
        if lo > hi {
            return Err(bad("range is backwards"));
        }
        let mut v = lo;
        while v <= hi {
            set |= 1 << v;
            v += step;
        }
    }
    Ok(set)
}

fn number(text: &str, min: u64, max: u64, bad: &dyn Fn(&str) -> Error) -> Result<u64, Error> {
    let n: u64 = text.parse().map_err(|_| bad("not a number"))?;
    if n < min || n > max {
        return Err(bad(&format!("out of range {min}-{max}")));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds since the epoch for a UTC date and time, via the same
    /// arithmetic in reverse; checked against known values below.
    fn at(y: u64, m: u64, d: u64, h: u64, min: u64) -> u64 {
        let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
        let era = y / 400;
        let yoe = y - era * 400;
        let doy = (153 * (m - 3) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * 86_400 + h * 3600 + min * 60
    }

    fn next(expr: &str, t: u64) -> u64 {
        Schedule::parse(expr)
            .expect(expr)
            .next_after(t)
            .expect("some time")
    }

    #[test]
    fn civil_round_trips_known_dates() {
        assert_eq!(at(1970, 1, 1, 0, 0), 0);
        assert_eq!(at(2023, 11, 14, 22, 13), 1_699_999_980);
        assert_eq!(civil(0), (1970, 1, 1, 4), "a Thursday");
        assert_eq!(
            civil(1_700_000_000 / 86_400),
            (2023, 11, 14, 2),
            "a Tuesday"
        );
        assert_eq!(civil(at(2024, 2, 29, 0, 0) / 86_400), (2024, 2, 29, 4));
    }

    #[test]
    fn every_minute_hourly_and_daily() {
        let t = at(2023, 11, 14, 22, 13) + 20;
        assert_eq!(next("* * * * *", t), at(2023, 11, 14, 22, 14));
        assert_eq!(next("*/15 * * * *", t), at(2023, 11, 14, 22, 15));
        assert_eq!(next("0 * * * *", t), at(2023, 11, 14, 23, 0));
        assert_eq!(next("30 9 * * *", t), at(2023, 11, 15, 9, 30));
        assert_eq!(
            next("30 9 * * *", at(2023, 11, 15, 9, 30)),
            at(2023, 11, 16, 9, 30),
            "strictly after"
        );
    }

    #[test]
    fn weekdays_month_ends_and_leap_days() {
        // 2023-11-17 is a Friday; weekdays only skips the weekend.
        assert_eq!(
            next("0 9 * * 1-5", at(2023, 11, 17, 10, 0)),
            at(2023, 11, 20, 9, 0)
        );
        assert_eq!(
            next("0 9 * * 0", at(2023, 11, 17, 10, 0)),
            at(2023, 11, 19, 9, 0)
        );
        assert_eq!(
            next("0 9 * * 7", at(2023, 11, 17, 10, 0)),
            at(2023, 11, 19, 9, 0),
            "7 is Sunday"
        );
        // The 31st: November has none.
        assert_eq!(
            next("0 0 31 * *", at(2023, 11, 1, 0, 0)),
            at(2023, 12, 31, 0, 0)
        );
        assert_eq!(
            next("0 0 29 2 *", at(2023, 3, 1, 0, 0)),
            at(2024, 2, 29, 0, 0)
        );
        // Both day fields set: either matches (the 1st, or any Monday).
        assert_eq!(
            next("0 0 1 * 1", at(2023, 11, 14, 0, 0)),
            at(2023, 11, 20, 0, 0)
        );
        assert_eq!(
            next("0 0 1 * 1", at(2023, 11, 27, 0, 0)),
            at(2023, 12, 1, 0, 0)
        );
        // 30 February never comes.
        assert_eq!(
            Schedule::parse("0 0 30 2 *").expect("parses").next_after(0),
            None
        );
    }

    #[test]
    fn bad_expressions_fail_to_parse() {
        for (expr, word) in [
            ("* * * *", "5 fields"),
            ("60 * * * *", "out of range"),
            ("* 24 * * *", "out of range"),
            ("* * 0 * *", "out of range"),
            ("* * * 13 *", "out of range"),
            ("* * * * 8", "out of range"),
            ("5-1 * * * *", "backwards"),
            ("*/0 * * * *", "positive"),
            ("a * * * *", "not a number"),
        ] {
            let err = Schedule::parse(expr).expect_err(expr).to_string();
            assert!(err.contains(word), "{expr}: {err} should mention {word}");
        }
        assert_eq!(
            next("5/10 * * * *", at(2023, 11, 14, 22, 13)),
            at(2023, 11, 14, 22, 15)
        );
    }
}
