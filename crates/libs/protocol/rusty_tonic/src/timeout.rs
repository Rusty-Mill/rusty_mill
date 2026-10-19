use std::time::Duration;

/// Why a `grpc-timeout` value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutError {
    /// No value, or no digits before the unit.
    Empty,
    /// More than 8 digits, or a non-digit before the unit.
    BadDigits,
    /// A unit other than `H M S m u n`.
    BadUnit,
}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TimeoutError::Empty => "empty grpc-timeout",
            TimeoutError::BadDigits => "grpc-timeout needs 1 to 8 digits",
            TimeoutError::BadUnit => "grpc-timeout unit must be one of H M S m u n",
        })
    }
}

impl std::error::Error for TimeoutError {}

/// Parses a `grpc-timeout` value: 1 to 8 digits and a unit (`H M S m u n`).
pub fn parse_timeout(value: &[u8]) -> Result<Duration, TimeoutError> {
    let (&unit, digits) = value.split_last().ok_or(TimeoutError::Empty)?;
    if !b"HMSmun".contains(&unit) {
        return Err(TimeoutError::BadUnit);
    }
    if digits.is_empty() {
        return Err(TimeoutError::Empty);
    }
    if digits.len() > 8 || !digits.iter().all(u8::is_ascii_digit) {
        return Err(TimeoutError::BadDigits);
    }
    let n = digits
        .iter()
        .fold(0u64, |n, d| n * 10 + u64::from(d - b'0'));
    Ok(match unit {
        b'H' => Duration::from_secs(n * 3600),
        b'M' => Duration::from_secs(n * 60),
        b'S' => Duration::from_secs(n),
        b'm' => Duration::from_millis(n),
        b'u' => Duration::from_micros(n),
        _ => Duration::from_nanos(n),
    })
}

/// Formats a `grpc-timeout` value in the finest unit that fits 8 digits, rounding up so the
/// deadline is never shortened. A duration past `99999999H` is clamped to it.
pub fn format_timeout(d: Duration) -> String {
    const UNITS: [(u128, char); 6] = [
        (1, 'n'),
        (1_000, 'u'),
        (1_000_000, 'm'),
        (1_000_000_000, 'S'),
        (60_000_000_000, 'M'),
        (3_600_000_000_000, 'H'),
    ];
    let nanos = d.as_nanos();
    UNITS
        .iter()
        .map(|&(size, unit)| (nanos.div_ceil(size), unit))
        .find(|&(n, _)| n <= 99_999_999)
        .map_or_else(|| "99999999H".to_string(), |(n, unit)| format!("{n}{unit}"))
}
