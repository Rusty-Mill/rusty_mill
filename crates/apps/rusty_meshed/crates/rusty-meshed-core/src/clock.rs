//! One wall-clock reading, fully converted up front.
//!
//! The only fallible step is [`ClockReading::now`] (or
//! [`ClockReading::from_system_time`]): it reads the system clock once and
//! finishes every conversion -- seconds to `i64`, milliseconds to `i64`,
//! the RFC 3339 [`Timestamp`] -- before returning. Everything derived
//! afterwards is an infallible accessor, so a publisher that reads a
//! reading before its first I/O cannot fail on time halfway through.
//!
//! Replaces the eight hand-rolled `now_iso()` formatters and the
//! `now_millis()` helpers that used `unwrap_or_default()` and `as` casts
//! (a pre-1970 clock silently became 1970-01-01).

use rusty_err::Error;
use rusty_time::DateTime;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Why a clock reading or timestamp could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClockError {
    /// The system clock reads before the Unix epoch -- far more likely a
    /// dead real-time clock than a real date.
    #[error("system clock is before the Unix epoch")]
    BeforeEpoch,
    /// The instant does not fit an `i64` count or RFC 3339's four-digit year.
    #[error("clock value is out of range")]
    OutOfRange,
    /// The text is not a whole-second UTC RFC 3339 timestamp.
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(&'static str),
}

/// A whole-second UTC RFC 3339 timestamp: always `YYYY-MM-DDTHH:MM:SSZ`
/// (20 bytes, years 0000..=9999).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(String);

impl Timestamp {
    const LEN: usize = 20;

    /// The timestamp `secs` seconds after the Unix epoch (negative before it).
    pub fn from_unix_secs(secs: i64) -> Result<Self, ClockError> {
        let dt = DateTime::from_unix_secs(secs).map_err(|_| ClockError::OutOfRange)?;
        if !(0..=9999).contains(&dt.date().year()) {
            return Err(ClockError::OutOfRange);
        }
        Ok(Timestamp(dt.to_iso8601()))
    }

    /// Parses exactly `YYYY-MM-DDTHH:MM:SSZ`. Fractional seconds, numeric
    /// offsets, lowercase `t`/`z`, a space separator, leap seconds and
    /// anything longer or shorter are rejected.
    pub fn parse(s: &str) -> Result<Self, ClockError> {
        let bad = ClockError::InvalidTimestamp;
        let b = s.as_bytes();
        if b.len() != Self::LEN {
            return Err(bad("expected exactly 20 bytes (YYYY-MM-DDTHH:MM:SSZ)"));
        }
        const SHAPE: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";
        let shaped = b.iter().zip(SHAPE).all(|(c, p)| match p {
            b'd' => c.is_ascii_digit(),
            lit => c == lit,
        });
        if !shaped {
            return Err(bad("expected the shape YYYY-MM-DDTHH:MM:SSZ"));
        }
        if &s[17..19] > "59" {
            return Err(bad("leap seconds are not representable"));
        }
        let dt = DateTime::parse(s).map_err(bad)?;
        if dt.time().nanosecond() != 0 || dt.offset_secs() != 0 || dt.to_iso8601() != s {
            return Err(bad("not a canonical whole-second UTC timestamp"));
        }
        Ok(Timestamp(s.to_string()))
    }

    /// The `YYYY-MM-DDTHH:MM:SSZ` text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One system-clock read with every derived value precomputed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockReading {
    since_epoch: Duration,
    unix_millis: i64,
    timestamp: Timestamp,
}

impl ClockReading {
    /// Reads the system clock once.
    pub fn now() -> Result<Self, ClockError> {
        Self::from_system_time(SystemTime::now())
    }

    /// Builds a reading from `time`; [`BeforeEpoch`](ClockError::BeforeEpoch)
    /// if it precedes 1970.
    pub fn from_system_time(time: SystemTime) -> Result<Self, ClockError> {
        let since_epoch = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ClockError::BeforeEpoch)?;
        Self::from_duration(since_epoch)
    }

    /// Builds a reading `since_epoch` after the Unix epoch.
    pub fn from_duration(since_epoch: Duration) -> Result<Self, ClockError> {
        let secs = i64::try_from(since_epoch.as_secs()).map_err(|_| ClockError::OutOfRange)?;
        let unix_millis =
            i64::try_from(since_epoch.as_millis()).map_err(|_| ClockError::OutOfRange)?;
        Ok(ClockReading {
            since_epoch,
            unix_millis,
            timestamp: Timestamp::from_unix_secs(secs)?,
        })
    }

    /// Whole seconds since the Unix epoch.
    pub fn unix_secs(&self) -> i64 {
        self.unix_millis.div_euclid(1000)
    }

    /// Milliseconds since the Unix epoch (Kafka record time).
    pub fn unix_millis(&self) -> i64 {
        self.unix_millis
    }

    /// Milliseconds since the Unix epoch with the sub-millisecond fraction,
    /// for the freshness lag. Evaluated as `as_secs_f64() * 1000.0`, the
    /// exact expression the pre-`ClockReading` code used.
    pub fn unix_millis_f64(&self) -> f64 {
        self.since_epoch.as_secs_f64() * 1000.0
    }

    /// The whole-second RFC 3339 timestamp (sub-second part truncated).
    pub fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}

/// Source of [`ClockReading`]s, so batch orchestration can be tested.
pub trait WallClock {
    /// Reads the clock once.
    fn read(&self) -> Result<ClockReading, ClockError>;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn read(&self) -> Result<ClockReading, ClockError> {
        ClockReading::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_SECS_9999: u64 = 253_402_300_799; // 9999-12-31T23:59:59Z

    fn reading(secs: u64, nanos: u32) -> Result<ClockReading, ClockError> {
        ClockReading::from_duration(Duration::new(secs, nanos))
    }

    #[test]
    fn parse_accepts_canonical_boundaries() {
        for s in [
            "0000-01-01T00:00:00Z",
            "9999-12-31T23:59:59Z",
            "2026-02-28T12:34:56Z",
        ] {
            assert_eq!(Timestamp::parse(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn parse_rejects_everything_but_whole_second_utc() {
        let cases = [
            "2026-10-06T12:00:05.5Z",
            "2026-10-06T12:00:05.000Z",
            "2026-10-06T12:00:05+00:00",
            "2026-10-06T12:00:05z",
            "2026-10-06t12:00:05Z",
            "2026-10-06 12:00:05Z",
            "2026-10-06T12:00:60Z",
            "2026-13-06T12:00:05Z",
            "2026-02-30T12:00:05Z",
            "2025-02-29T12:00:05Z",
            "2026-10-06T24:00:05Z",
            "10000-10-06T12:00:05Z",
            "-001-10-06T12:00:05Z",
            "",
            "２０２６-10-06T12:00:05Z",
            "2026-10-06T12:00:05Z\n",
            "2026-10-06T12:00:05ZZ",
        ];
        for s in cases {
            assert!(
                matches!(Timestamp::parse(s), Err(ClockError::InvalidTimestamp(_))),
                "accepted {s:?}"
            );
        }
    }

    #[test]
    fn parse_accepts_a_leap_day() {
        assert!(Timestamp::parse("2024-02-29T00:00:00Z").is_ok());
    }

    #[test]
    fn from_unix_secs_covers_pre_epoch_and_year_limits() {
        assert_eq!(
            Timestamp::from_unix_secs(0).unwrap().as_str(),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            Timestamp::from_unix_secs(-1).unwrap().as_str(),
            "1969-12-31T23:59:59Z"
        );
        let max = i64::try_from(MAX_SECS_9999).unwrap();
        assert_eq!(
            Timestamp::from_unix_secs(max).unwrap().as_str(),
            "9999-12-31T23:59:59Z"
        );
        assert_eq!(
            Timestamp::from_unix_secs(max + 1),
            Err(ClockError::OutOfRange)
        );
        assert_eq!(
            Timestamp::from_unix_secs(-62_167_219_200).unwrap().as_str(),
            "0000-01-01T00:00:00Z"
        );
        assert_eq!(
            Timestamp::from_unix_secs(-62_167_219_201),
            Err(ClockError::OutOfRange)
        );
    }

    #[test]
    fn output_always_reparses() {
        for secs in [0, 86_399, 951_782_400, 1_700_000_000, 4_102_444_800] {
            let ts = Timestamp::from_unix_secs(secs).unwrap();
            assert_eq!(Timestamp::parse(ts.as_str()).unwrap(), ts);
        }
    }

    #[test]
    fn reading_truncates_the_subsecond_part_and_keeps_millis() {
        let r = reading(1_700_000_000, 999_999_999).unwrap();
        assert_eq!(r.timestamp().as_str(), "2023-11-14T22:13:20Z");
        assert_eq!(r.unix_millis(), 1_700_000_000_999);
        assert_eq!(r.unix_secs(), 1_700_000_000);
    }

    #[test]
    fn reading_rejects_pre_epoch_and_out_of_range() {
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(
            ClockReading::from_system_time(before),
            Err(ClockError::BeforeEpoch)
        );
        assert_eq!(reading(u64::MAX, 0), Err(ClockError::OutOfRange));
        assert_eq!(reading(i64::MAX as u64 + 1, 0), Err(ClockError::OutOfRange));
        assert_eq!(reading(MAX_SECS_9999 + 1, 0), Err(ClockError::OutOfRange));
        assert!(reading(MAX_SECS_9999, 999_999_999).is_ok());
    }

    #[test]
    fn millis_overflow_is_out_of_range() {
        // seconds fit i64 but seconds * 1000 does not
        assert_eq!(reading(i64::MAX as u64 / 2, 0), Err(ClockError::OutOfRange));
    }

    /// The freshness lag must not change by a single bit.
    #[test]
    fn unix_millis_f64_matches_the_old_expression_bit_for_bit() {
        let old = |d: Duration| d.as_secs_f64() * 1000.0;
        let cases = [
            Duration::ZERO,
            Duration::new(0, 1),
            Duration::new(0, 999_999),
            Duration::new(1, 500_000),
            Duration::new(1_700_000_000, 123_456_789),
            Duration::new(MAX_SECS_9999, 999_999_999),
        ];
        for d in cases {
            let r = ClockReading::from_duration(d).unwrap();
            assert_eq!(r.unix_millis_f64().to_bits(), old(d).to_bits(), "{d:?}");
            let ts = 1_699_999_990_000_i64;
            let new_lag = (r.unix_millis_f64() - ts as f64) / 1000.0;
            let old_lag = (old(d) - ts as f64) / 1000.0;
            assert_eq!(new_lag.to_bits(), old_lag.to_bits(), "{d:?}");
        }
    }
}
