//! The methods of the terminal block type, `terminal.*`, over a block's grid.

use std::path::Path;

use rusty_mux_status::Records;
use rusty_term_core::{Grid, StatusSignal};

/// The last `history` scrollback lines and the screen, one line per row
/// without trailing blanks, joined by `\n`. Stops once the text passes
/// `max` bytes; the caller refuses a reply that large.
pub(crate) fn format(grid: &Grid, history: u32, max: usize) -> String {
    let scrollback = grid.scrollback_len();
    let from = scrollback.saturating_sub(usize::try_from(history).unwrap_or(usize::MAX));
    let mut text = String::new();
    for abs in from..scrollback + grid.rows {
        if abs > from {
            text.push('\n');
        }
        text.push_str(&grid.line_text(abs).unwrap_or_default());
        if text.len() > max {
            break;
        }
    }
    text
}

/// The program's working directory: the one it last reported with OSC 7,
/// else the current directory of process `pid` (the program the block
/// started; a job it runs may be elsewhere). `None` if neither is known or
/// the path is not UTF-8.
pub(crate) fn pwd(grid: &Grid, pid: Option<u32>) -> Option<String> {
    reported_path(&grid.cwd).or_else(|| {
        let link = std::fs::read_link(Path::new("/proc").join(pid?.to_string()).join("cwd"));
        link.ok()?.into_os_string().into_string().ok()
    })
}

/// Applies the status signals the grid collected since the last call: OSC
/// 7501 reports, prompts and resets. Whether any record's state changed, the
/// part worth announcing; progress and text alone are not.
pub(crate) fn absorb_status(grid: &mut Grid, records: &mut Records) -> bool {
    if grid.program_status.is_empty() {
        return false;
    }
    let before = records.states();
    for signal in grid.program_status.drain(..) {
        match signal {
            StatusSignal::Report(body) => {
                if let Some(report) = rusty_mux_status::parse(&body) {
                    records.apply(report);
                }
            }
            StatusSignal::Prompt => records.prompt(),
            StatusSignal::Reset => records.reset(),
        }
    }
    records.states() != before
}

/// The working directory URI (OSC 7) and title a block last announced, to
/// tell when either changes.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    cwd: String,
    title: String,
}

impl Seen {
    /// Whether the grid's working directory and title, in that order,
    /// changed since the last call.
    pub(crate) fn changes(&mut self, grid: &Grid) -> (bool, bool) {
        (
            update(&mut self.cwd, &grid.cwd),
            update(&mut self.title, &grid.title),
        )
    }
}

/// Makes `seen` `now`; whether it differed.
fn update(seen: &mut String, now: &str) -> bool {
    if seen == now {
        return false;
    }
    now.clone_into(seen);
    true
}

/// The path of an OSC 7 `file://HOST/PATH` URI, percent-decoded.
fn reported_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    String::from_utf8(percent_decode(path)?).ok()
}

/// `None` on a `%` not followed by two hex digits.
fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b != b'%' {
            out.push(b);
            i += 1;
            continue;
        }
        let hex = bytes.get(i + 1..i + 3)?;
        if !hex.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
        i += 3;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_paths_are_decoded() {
        assert_eq!(
            reported_path("file://host/home/a%20b/%E4%B8%AD"),
            Some("/home/a b/\u{4e2d}".into())
        );
        assert_eq!(reported_path("file:///tmp"), Some("/tmp".into()));
    }

    #[test]
    fn malformed_reports_are_ignored() {
        for uri in [
            "",
            "/tmp",
            "http://host/tmp",
            "file://host",
            "file:///a%2",
            "file:///a%zz",
            "file:///%ff",
            "file:///a%+1",
        ] {
            assert_eq!(reported_path(uri), None, "{uri}");
        }
    }

    #[test]
    fn pwd_and_title_changes_are_seen_once() {
        let mut grid = Grid::new(20, 2);
        let mut parser = rusty_term_core::AnsiParser::new();
        let mut seen = Seen::default();
        assert_eq!(seen.changes(&grid), (false, false), "nothing yet");
        parser.advance(&mut grid, b"\x1b]7;file://host/tmp\x07\x1b]2;job\x07");
        assert_eq!(seen.changes(&grid), (true, true));
        assert_eq!(seen.changes(&grid), (false, false), "announced once");
        parser.advance(&mut grid, b"\x1b]7;file://host/tmp\x07\x1b]2;next\x07");
        assert_eq!(seen.changes(&grid), (false, true), "same directory");
    }

    fn status_grid() -> (Grid, rusty_term_core::AnsiParser, Records) {
        let mut grid = Grid::new(20, 2);
        grid.collect_program_status = true;
        (grid, rusty_term_core::AnsiParser::new(), Records::default())
    }

    #[test]
    fn status_changes_are_announced_once() {
        let (mut grid, mut parser, mut records) = status_grid();
        assert!(!absorb_status(&mut grid, &mut records), "nothing collected");
        parser.advance(&mut grid, b"\x1b]7501;state=working:progress=10\x07");
        assert!(absorb_status(&mut grid, &mut records));
        parser.advance(&mut grid, b"\x1b]7501;state=working:progress=90\x07");
        assert!(!absorb_status(&mut grid, &mut records), "progress only");
        parser.advance(&mut grid, b"\x1b]7501;state=bogus\x07");
        assert!(!absorb_status(&mut grid, &mut records), "ignored report");
        assert!(grid.program_status.is_empty(), "drained");
        assert_eq!(records.bodies(), ["state=working:progress=90"]);
    }

    #[test]
    fn prompts_and_resets_end_records() {
        let (mut grid, mut parser, mut records) = status_grid();
        parser.advance(&mut grid, b"\x1b]7501;state=blocked:kind=question\x07");
        parser.advance(&mut grid, b"\x1b]7501;state=done:id=build\x07");
        assert!(absorb_status(&mut grid, &mut records));
        parser.advance(&mut grid, b"\x1b]133;A\x07");
        assert!(absorb_status(&mut grid, &mut records));
        assert_eq!(records.bodies(), ["state=done:id=build"]);
        parser.advance(&mut grid, b"\x1bc");
        assert!(absorb_status(&mut grid, &mut records));
        assert!(records.bodies().is_empty());
    }

    #[test]
    fn a_report_after_an_overflowing_burst_still_lands() {
        let (mut grid, mut parser, mut records) = status_grid();
        let round = b"\x1b]7501;state=working:id=a\x07\x1b]133;A\x07";
        let burst = round.repeat(rusty_term_core::PROGRAM_STATUS_MAX * 2);
        parser.advance(&mut grid, &burst);
        parser.advance(&mut grid, b"\x1b]7501;state=done\x07");
        assert!(absorb_status(&mut grid, &mut records));
        assert_eq!(records.bodies(), ["state=done"]);
    }

    #[test]
    fn format_stops_past_the_limit() {
        let mut grid = Grid::new(4, 3);
        let mut parser = rusty_term_core::AnsiParser::new();
        parser.advance(&mut grid, b"abcd\r\nefgh\r\nijkl");
        assert_eq!(format(&grid, 0, usize::MAX), "abcd\nefgh\nijkl");
        assert_eq!(format(&grid, 0, 5), "abcd\nefgh");
    }
}
