//! A Rust port of `server/extractor/extractors/ytdlp/format.go`.

/// Converts seconds to a human-readable `HH:MM:SS` or `MM:SS` string.
pub(super) fn format_duration(seconds: f64) -> String {
    let total = seconds as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Converts `YYYYMMDD` to `YYYY-MM-DD`.
pub(super) fn format_date(d: &str) -> String {
    if d.len() == 8 {
        format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..])
    } else {
        d.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration_under_an_hour_as_minutes_and_seconds() {
        assert_eq!(format_duration(65.0), "1:05");
    }

    #[test]
    fn formats_duration_over_an_hour_with_hours() {
        assert_eq!(format_duration(3725.0), "1:02:05");
    }

    #[test]
    fn formats_upload_date() {
        assert_eq!(format_date("20260812"), "2026-08-12");
    }

    #[test]
    fn leaves_a_malformed_date_unchanged() {
        assert_eq!(format_date("not-a-date"), "not-a-date");
    }
}
