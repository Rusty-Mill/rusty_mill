//! Human-readable rendering shared by the CLI and the desktop shell.

use crate::model::now_unix;

/// `2026-08-05 12:00` in UTC.
pub fn timestamp(unix: i64) -> String {
    if unix <= 0 {
        return "unknown".into();
    }
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    let secs = unix.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

pub fn date(unix: i64) -> String {
    if unix <= 0 {
        return "unknown".into();
    }
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// "31m ago", "10h ago", "2d ago" — the shape the product's own result list
/// uses.
pub fn relative(unix: i64) -> String {
    if unix <= 0 {
        return "unknown".into();
    }
    let delta = now_unix() - unix;
    if delta < 0 {
        return "just now".into();
    }
    match delta {
        d if d < 60 => "just now".into(),
        d if d < 3600 => format!("{}m ago", d / 60),
        d if d < 86_400 => format!("{}h ago", d / 3600),
        d if d < 86_400 * 30 => format!("{}d ago", d / 86_400),
        d if d < 86_400 * 365 => format!("{}mo ago", d / (86_400 * 30)),
        d => format!("{}y ago", d / (86_400 * 365)),
    }
}

pub fn bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Inverse of the civil-days algorithm in `sources`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_known_instant() {
        assert_eq!(timestamp(1_785_931_200), "2026-08-05 12:00");
        assert_eq!(date(1_785_931_200), "2026-08-05");
        assert_eq!(timestamp(0), "unknown");
    }

    #[test]
    fn round_trips_against_the_forward_calendar() {
        // Agreement with the parser used when reading transcripts.
        // Epoch zero is excluded on purpose: parsers use 0 as the "no
        // timestamp recorded" sentinel, which renders as "unknown".
        for iso in [
            "1970-01-02T00:00:00Z",
            "2000-02-29T23:59:00Z",
            "2026-08-05T12:00:00Z",
            "2031-12-31T00:00:00Z",
        ] {
            let parsed =
                crate::sources::parse_timestamp(&serde_json::Value::String(iso.into())).unwrap();
            assert_eq!(&timestamp(parsed), &iso[..16].replace('T', " "), "{iso}");
        }
    }

    #[test]
    fn relative_times_read_the_way_the_result_list_does() {
        let now = now_unix();
        assert_eq!(relative(now - 10), "just now");
        assert_eq!(relative(now - 31 * 60), "31m ago");
        assert_eq!(relative(now - 10 * 3600), "10h ago");
        assert_eq!(relative(now - 2 * 86_400), "2d ago");
    }

    #[test]
    fn byte_sizes_are_readable() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2048), "2.0 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
