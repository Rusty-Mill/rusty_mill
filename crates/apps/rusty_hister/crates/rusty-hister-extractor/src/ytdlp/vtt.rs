//! A Rust port of `server/extractor/extractors/ytdlp/vtt.go`: extracts
//! plain transcript text from WebVTT subtitle content.

/// Extracts plain text lines from WebVTT content, stripping headers,
/// timestamps, and deduplicating repeated lines (a real WebVTT quirk:
/// yt-dlp's auto-captions repeat each line across several overlapping
/// cues for scroll animation).
pub(super) fn parse_vtt(raw: &str) -> String {
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in raw.split('\n') {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("WEBVTT")
            || line.starts_with("NOTE")
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.contains(" --> ")
            || is_numeric(line)
        {
            continue;
        }
        let clean = strip_vtt_tags(line);
        let clean = clean.trim();
        if clean.is_empty() {
            continue;
        }
        if seen.insert(clean.to_string()) {
            lines.push(clean.to_string());
        }
    }

    lines.join(" ")
}

/// Reports whether `s` consists entirely of digits (WebVTT cue identifiers).
fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Removes VTT/HTML-style tags (`<c>`, `</c>`, `<00:00:01.234>`, ...) from text.
fn strip_vtt_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_headers_timestamps_and_cue_numbers() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n1\n00:00:00.000 --> 00:00:02.000\nHello there\n\n2\n00:00:02.000 --> 00:00:04.000\nGeneral Kenobi\n";
        assert_eq!(parse_vtt(vtt), "Hello there General Kenobi");
    }

    #[test]
    fn strips_inline_tags_and_deduplicates_repeated_lines() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n<c>Hello</c> <00:00:01.000>there\n\n00:00:01.000 --> 00:00:03.000\n<c>Hello</c> <00:00:01.000>there\n";
        assert_eq!(parse_vtt(vtt), "Hello there");
    }

    #[test]
    fn ignores_note_blocks() {
        let vtt =
            "WEBVTT\n\nNOTE this is a comment\n\n00:00:00.000 --> 00:00:01.000\nActual text\n";
        assert_eq!(parse_vtt(vtt), "Actual text");
    }
}
