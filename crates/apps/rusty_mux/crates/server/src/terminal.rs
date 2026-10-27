//! The methods of the terminal block type, `terminal.*`, over a block's grid.

use std::path::Path;

use rusty_mux_core::Progress;
use rusty_mux_status::Records;
use rusty_term_core::{
    ATTR_BLINK, ATTR_BOLD, ATTR_DIM, ATTR_HIDDEN, ATTR_ITALIC, ATTR_MASK, ATTR_REVERSE,
    ATTR_STRIKE, ATTR_UNDERLINE, ATTR_UNDERLINE_COLOR, Cell, Grid, StatusSignal, UnderlineStyle,
    WIDE_TRAILER,
};

/// What `terminal.format` reads, and how.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capture {
    /// Scrollback lines above the screen to include.
    pub(crate) history: u32,
    /// Leave out blank lines at either end.
    pub(crate) trim: bool,
    /// Join soft-wrapped rows into one line.
    pub(crate) unwrap: bool,
    /// Keep styles as SGR sequences.
    pub(crate) vt: bool,
}

/// The last `history` scrollback lines and the screen, one line per row
/// (per wrapped run with `unwrap`) without trailing blanks, joined by `\n`.
/// A blank left where a wide glyph did not fit is kept: the grid stores it
/// as it does a typed space, and dropping a typed one would lose text.
/// `trim` leaves out blank lines at either end. `None` if the text,
/// trimmed, would pass `max` bytes: a capture is whole or refused, never
/// cut short.
pub(crate) fn format(grid: &Grid, capture: Capture, max: usize) -> Option<String> {
    let scrollback = grid.scrollback_len();
    let history = usize::try_from(capture.history).unwrap_or(usize::MAX);
    let end = scrollback + grid.rows;
    let mut text = String::new();
    let mut lines = 0usize;
    // Blank lines `trim` holds back until a line with text follows.
    let mut held = 0usize;
    let mut run: Vec<Cell> = Vec::new();
    for abs in scrollback.saturating_sub(history)..end {
        let Some((mut cells, wrapped)) = grid.line_cells(abs) else {
            break;
        };
        if capture.unwrap && wrapped && abs + 1 < end {
            cells.resize(grid.cols.max(cells.len()), Cell::blank());
            run.append(&mut cells);
            continue;
        }
        run.append(&mut cells);
        let line = line(&run, &grid.clusters, capture.vt);
        run.clear();
        if capture.trim && line.is_empty() {
            // Leading blanks are dropped, later ones held.
            held += usize::from(lines > 0);
            continue;
        }
        for _ in 0..held + usize::from(lines > 0) {
            text.push('\n');
        }
        lines += held + 1;
        held = 0;
        text.push_str(&line);
        if text.len() > max {
            return None;
        }
    }
    Some(text)
}

/// `cells` as text without trailing blanks; with `vt`, each change of
/// style as one SGR sequence that resets first, so a non-empty line opens
/// with one even in the default style and ends in the default style; it
/// prints the same under any pen.
fn line(cells: &[Cell], clusters: &[String], vt: bool) -> String {
    let len = match vt {
        true => cells
            .iter()
            .rposition(|c| !is_padding(c))
            .map_or(0, |i| i + 1),
        false => cells.len(),
    };
    let blank = Style::of(&Cell::blank());
    // `None` until the first cell, which always sets its style.
    let mut style = None;
    let mut text = String::new();
    for cell in cells[..len].iter().filter(|c| !is_trailer(c)) {
        if vt && style != Some(Style::of(cell)) {
            let now = Style::of(cell);
            now.sgr(&mut text);
            style = Some(now);
        }
        text.push(cell.ch);
        if let Some(rest) = usize::from(cell.cluster)
            .checked_sub(1)
            .and_then(|i| clusters.get(i))
        {
            text.push_str(rest);
        }
    }
    if style.is_some_and(|s| s != blank) {
        text.push_str("\x1b[0m");
    }
    if !vt {
        text.truncate(text.trim_end().len());
    }
    text
}

/// A blank in the default style, which ends no line.
fn is_padding(cell: &Cell) -> bool {
    let blank = Cell::blank();
    cell.ch == ' ' && cell.cluster == 0 && cell.link == 0 && Style::of(cell) == Style::of(&blank)
}

/// The second cell of a wide glyph, which holds no text.
fn is_trailer(cell: &Cell) -> bool {
    cell.flags & WIDE_TRAILER != 0
}

/// What a cell's SGR sequence sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: u32,
    bg: u32,
    attrs: u16,
    /// Only with [`ATTR_UNDERLINE_COLOR`].
    underline: Option<u32>,
}

impl Style {
    fn of(cell: &Cell) -> Self {
        let attrs = cell.flags & ATTR_MASK;
        Self {
            fg: cell.fg,
            bg: cell.bg,
            attrs,
            underline: (attrs & ATTR_UNDERLINE_COLOR != 0).then_some(cell.underline_color),
        }
    }

    /// One SGR sequence that resets, then sets this style.
    fn sgr(self, out: &mut String) {
        let blank = Cell::blank();
        out.push_str("\x1b[0");
        for (bit, code) in [
            (ATTR_BOLD, "1"),
            (ATTR_DIM, "2"),
            (ATTR_ITALIC, "3"),
            (ATTR_BLINK, "5"),
            (ATTR_REVERSE, "7"),
            (ATTR_HIDDEN, "8"),
            (ATTR_STRIKE, "9"),
        ] {
            if self.attrs & bit != 0 {
                out.push(';');
                out.push_str(code);
            }
        }
        if self.attrs & ATTR_UNDERLINE != 0 {
            out.push_str(match UnderlineStyle::from_attrs(self.attrs) {
                UnderlineStyle::Straight => ";4",
                UnderlineStyle::Double => ";4:2",
                UnderlineStyle::Curly => ";4:3",
                UnderlineStyle::Dotted => ";4:4",
                UnderlineStyle::Dashed => ";4:5",
            });
        }
        if self.fg != blank.fg {
            push_rgb(out, 38, self.fg);
        }
        if self.bg != blank.bg {
            push_rgb(out, 48, self.bg);
        }
        if let Some(rgb) = self.underline {
            push_rgb(out, 58, rgb);
        }
        out.push('m');
    }
}

fn push_rgb(out: &mut String, code: u8, rgb: u32) {
    let [_, r, g, b] = rgb.to_be_bytes();
    out.push_str(&format!(";{code};2;{r};{g};{b}"));
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
    /// OSC 9;4's `(state, percent)`, as the grid keeps it.
    progress: Option<(u8, u8)>,
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

    /// The grid's progress, if it changed since the last call.
    pub(crate) fn progress(&mut self, grid: &Grid) -> Option<Progress> {
        if self.progress == grid.progress {
            return None;
        }
        self.progress = grid.progress;
        Some(match grid.progress {
            Some((1, p)) => Progress::Normal(p),
            Some((2, p)) => Progress::Error(p),
            Some((3, _)) => Progress::Indeterminate,
            Some((4, p)) => Progress::Paused(p),
            _ => Progress::Cleared,
        })
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

    #[test]
    fn progress_changes_are_seen_once() {
        let mut grid = Grid::new(20, 2);
        let mut parser = rusty_term_core::AnsiParser::new();
        let mut seen = Seen::default();
        assert_eq!(seen.progress(&grid), None, "nothing yet");
        let steps: [(&[u8], Option<Progress>); 6] = [
            (b"\x1b]9;4;1;40\x07", Some(Progress::Normal(40))),
            (b"\x1b]9;4;1;40\x07", None),
            (b"\x1b]9;4;2;150\x07", Some(Progress::Error(100))),
            (b"\x1b]9;4;3\x07", Some(Progress::Indeterminate)),
            (b"\x1b]9;4;4;7\x07", Some(Progress::Paused(7))),
            (b"\x1b]9;4;0\x07", Some(Progress::Cleared)),
        ];
        for (input, expected) in steps {
            parser.advance(&mut grid, input);
            assert_eq!(seen.progress(&grid), expected, "{input:?}");
        }
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

    fn screen(cols: usize, rows: usize, input: &[u8]) -> Grid {
        let mut grid = Grid::new(cols, rows);
        rusty_term_core::AnsiParser::new().advance(&mut grid, input);
        grid
    }

    fn capture(history: u32) -> Capture {
        Capture {
            history,
            ..Capture::default()
        }
    }

    #[test]
    fn format_stops_past_the_limit() {
        let grid = screen(4, 3, b"abcd\r\nefgh\r\nijkl");
        assert_eq!(
            format(&grid, capture(0), usize::MAX).expect("within the limit"),
            "abcd\nefgh\nijkl"
        );
        assert_eq!(
            format(&grid, capture(0), 14),
            Some("abcd\nefgh\nijkl".into())
        );
        assert_eq!(format(&grid, capture(0), 13), None, "whole or nothing");
    }

    #[test]
    fn trim_counts_only_what_it_keeps() {
        let grid = screen(4, 6, b"\r\n\r\n\r\nabc\r\ndef\r\nxyz");
        let trim = Capture {
            trim: true,
            ..capture(0)
        };
        assert_eq!(format(&grid, trim, 11), Some("abc\ndef\nxyz".into()));
        assert_eq!(format(&grid, trim, 10), None, "never a prefix");
        assert_eq!(
            format(&grid, capture(0), 11),
            None,
            "untrimmed it is larger"
        );
        let all = Capture {
            unwrap: true,
            vt: true,
            ..trim
        };
        let whole = format(&grid, all, usize::MAX).expect("within the limit");
        assert_eq!(whole, "\x1b[0mabc\n\x1b[0mdef\n\x1b[0mxyz");
        assert_eq!(format(&grid, all, whole.len()), Some(whole.clone()));
        assert_eq!(format(&grid, all, whole.len() - 1), None);
    }

    #[test]
    fn format_includes_the_history_asked_for() {
        let grid = screen(4, 2, b"a\r\nb\r\nc\r\nd");
        assert_eq!(
            format(&grid, capture(0), usize::MAX).expect("within the limit"),
            "c\nd"
        );
        assert_eq!(
            format(&grid, capture(1), usize::MAX).expect("within the limit"),
            "b\nc\nd"
        );
        assert_eq!(
            format(&grid, capture(u32::MAX), usize::MAX).expect("within the limit"),
            "a\nb\nc\nd"
        );
    }

    #[test]
    fn trim_drops_blank_lines_at_either_end() {
        let grid = screen(4, 5, b"\r\nab\r\n\r\ncd");
        let trim = Capture {
            trim: true,
            ..capture(0)
        };
        assert_eq!(
            format(&grid, capture(0), usize::MAX).expect("within the limit"),
            "\nab\n\ncd\n"
        );
        assert_eq!(
            format(&grid, trim, usize::MAX).expect("within the limit"),
            "ab\n\ncd"
        );
        assert_eq!(
            format(&screen(4, 2, b""), trim, usize::MAX).expect("within the limit"),
            ""
        );
    }

    #[test]
    fn unwrap_joins_wrapped_rows_and_keeps_their_blanks() {
        // "ab c" wraps after "ab c" at width 4; "d" ends the line.
        let grid = screen(4, 3, b"ab cd\r\nx");
        let unwrap = Capture {
            unwrap: true,
            ..capture(0)
        };
        assert_eq!(
            format(&grid, capture(0), usize::MAX).expect("within the limit"),
            "ab c\nd\nx"
        );
        assert_eq!(
            format(&grid, unwrap, usize::MAX).expect("within the limit"),
            "ab cd\nx"
        );
        let spaced = screen(4, 2, b"ab  cd");
        assert_eq!(
            format(&spaced, unwrap, usize::MAX).expect("within the limit"),
            "ab  cd"
        );
    }

    #[test]
    fn unwrap_keeps_the_blank_before_a_wrapped_wide_glyph() {
        let unwrap = Capture {
            unwrap: true,
            ..capture(0)
        };
        let vt = Capture { vt: true, ..unwrap };
        // A typed space fills column 3, then the wide glyph wraps.
        let typed = screen(4, 2, "abc \u{4e2d}".as_bytes());
        assert_eq!(
            format(&typed, unwrap, usize::MAX).expect("within the limit"),
            "abc \u{4e2d}"
        );
        // The glyph does not fit in column 3 and wraps; the blank it left
        // is stored as a typed space would be, so it is kept too.
        let skipped = screen(4, 2, "abc\u{4e2d}".as_bytes());
        assert_eq!(
            format(&skipped, unwrap, usize::MAX).expect("within the limit"),
            "abc \u{4e2d}"
        );
        // A styled blank is kept, with its style.
        let styled = screen(4, 2, "abc\x1b[41m \x1b[0m\u{4e2d}".as_bytes());
        assert_eq!(
            format(&styled, unwrap, usize::MAX).expect("within the limit"),
            "abc \u{4e2d}"
        );
        let text = format(&styled, vt, usize::MAX).expect("within the limit");
        assert!(text.starts_with("\x1b[0mabc\x1b[0;48;2;"), "{text:?}");
        assert!(text.ends_with(" \x1b[0m\u{4e2d}"), "{text:?}");
    }

    #[test]
    fn vt_keeps_styles_per_line() {
        let grid = screen(
            8,
            2,
            b"\x1b[1;31mab\x1b[0m c\r\n\x1b[4:3;48;2;1;2;3mx\x1b[0m",
        );
        let vt = Capture {
            vt: true,
            ..capture(0)
        };
        let text = format(&grid, vt, usize::MAX).expect("within the limit");
        let mut lines = text.lines();
        let first = lines.next().expect("first line");
        assert!(first.starts_with("\x1b[0;1;38;2;"), "{first:?}");
        assert!(first.ends_with("mab\x1b[0m c"), "{first:?}");
        assert_eq!(lines.next(), Some("\x1b[0;4:3;48;2;1;2;3mx\x1b[0m"));
        let plain = format(&grid, capture(0), usize::MAX).expect("within the limit");
        assert_eq!(plain, "ab c\nx");
    }

    #[test]
    fn vt_lines_print_the_same_under_any_pen() {
        let source = screen(8, 3, b"abc\r\nab\x1b[1;32mcd\x1b[0m\r\n\x1b[44mxy");
        let vt = Capture {
            vt: true,
            ..capture(0)
        };
        let text = format(&source, vt, usize::MAX).expect("within the limit");
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "\x1b[0mabc");
        assert!(lines[1].starts_with("\x1b[0mab\x1b[0;1;"), "{text:?}");
        assert!(
            lines
                .iter()
                .all(|l| l.ends_with("\x1b[0m") || !l.contains("\x1b[0;"))
        );
        // Replayed under a bold red pen, each line reproduces its source row.
        for (y, line) in lines.iter().enumerate() {
            let mut replay = Vec::from(&b"\x1b[1;31;47m"[..]);
            replay.extend_from_slice(line.as_bytes());
            let copy = screen(8, 1, &replay);
            let n = line.chars().filter(|c| c.is_alphabetic()).count();
            let source_row = source.line_cells(y).expect("source row").0;
            let copy_row = copy.line_cells(0).expect("copy row").0;
            let styles = |row: &[Cell]| row.iter().take(n).map(Style::of).collect::<Vec<_>>();
            assert_eq!(styles(&copy_row), styles(&source_row), "row {y}: {line:?}");
        }
    }

    #[test]
    fn vt_keeps_a_colored_blank_at_the_end() {
        let grid = screen(4, 1, b"a\x1b[41m \x1b[0m");
        let vt = Capture {
            vt: true,
            ..capture(0)
        };
        let text = format(&grid, vt, usize::MAX).expect("within the limit");
        assert!(
            text.starts_with("\x1b[0ma") && text.ends_with(" \x1b[0m"),
            "{text:?}"
        );
    }
}
