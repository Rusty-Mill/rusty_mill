//! `grpc-timeout`: 1 to 8 digits and a unit.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::Lcg;
use rusty_tonic::{format_timeout, parse_timeout, TimeoutError};

#[test]
fn each_unit_parses() {
    for (text, want) in [
        ("1H", Duration::from_secs(3600)),
        ("10M", Duration::from_secs(600)),
        ("5S", Duration::from_secs(5)),
        ("100m", Duration::from_millis(100)),
        ("250u", Duration::from_micros(250)),
        ("7n", Duration::from_nanos(7)),
        ("0n", Duration::ZERO),
        ("00000001S", Duration::from_secs(1)),
        ("99999999H", Duration::from_secs(99_999_999 * 3600)),
        ("99999999n", Duration::from_nanos(99_999_999)),
    ] {
        assert_eq!(parse_timeout(text.as_bytes()), Ok(want), "{text}");
    }
}

#[test]
fn bad_values_say_why() {
    for (bytes, want) in [
        (&b""[..], TimeoutError::Empty),
        (b"S", TimeoutError::Empty),
        (b"123456789S", TimeoutError::BadDigits),
        (b"-1S", TimeoutError::BadDigits),
        (b"1.5S", TimeoutError::BadDigits),
        (b" 1S", TimeoutError::BadDigits),
        (b"1 S", TimeoutError::BadDigits),
        (b"+1S", TimeoutError::BadDigits),
        ("\u{ff11}S".as_bytes(), TimeoutError::BadDigits),
        (b"1", TimeoutError::BadUnit),
        (b"1x", TimeoutError::BadUnit),
        (b"1h", TimeoutError::BadUnit),
        (b"1s", TimeoutError::BadUnit),
        (b"1SS", TimeoutError::BadDigits),
        (b"1\xff", TimeoutError::BadUnit),
    ] {
        assert_eq!(parse_timeout(bytes), Err(want), "{bytes:?}");
    }
}

#[test]
fn formatting_picks_the_finest_unit_that_fits_eight_digits() {
    for (d, want) in [
        (Duration::ZERO, "0n"),
        (Duration::from_nanos(1), "1n"),
        (Duration::from_nanos(99_999_999), "99999999n"),
        (Duration::from_nanos(100_000_000), "100000u"),
        (Duration::from_secs(1), "1000000u"),
        (Duration::from_millis(250), "250000u"),
        (Duration::from_secs(100), "100000m"),
        (Duration::from_secs(3600), "3600000m"),
        (Duration::from_secs(365 * 24 * 3600), "31536000S"),
    ] {
        assert_eq!(format_timeout(d), want, "{d:?}");
    }
}

#[test]
fn formatting_rounds_up_never_down() {
    assert_eq!(format_timeout(Duration::new(0, 100_000_001)), "100001u");
    assert_eq!(format_timeout(Duration::new(100, 1)), "100001m");
}

#[test]
fn a_huge_duration_is_clamped() {
    assert_eq!(format_timeout(Duration::MAX), "99999999H");
    assert_eq!(
        format_timeout(Duration::from_secs(99_999_999 * 3600 + 1)),
        "99999999H"
    );
}

#[test]
fn format_then_parse_never_shortens_and_stays_within_one_unit() {
    let mut rng = Lcg(3);
    for _ in 0..50_000 {
        let magnitude = 1u64 << rng.below(52);
        let d = Duration::from_nanos(rng.next() % magnitude);
        let text = format_timeout(d);
        assert!(text.len() <= 9, "{text}");
        let back = parse_timeout(text.as_bytes()).unwrap();
        assert!(back >= d, "{d:?} -> {text} -> {back:?}");
        let unit = match text.as_bytes()[text.len() - 1] {
            b'n' => Duration::from_nanos(1),
            b'u' => Duration::from_micros(1),
            b'm' => Duration::from_millis(1),
            b'S' => Duration::from_secs(1),
            b'M' => Duration::from_secs(60),
            _ => Duration::from_secs(3600),
        };
        assert!(
            back - d < unit,
            "{d:?} -> {text} rounds by more than a unit"
        );
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(4);
    for _ in 0..50_000 {
        let _ = parse_timeout(&rng.bytes(12));
    }
}
