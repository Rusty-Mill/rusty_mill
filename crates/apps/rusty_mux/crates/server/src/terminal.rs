//! The methods of the terminal block type, `terminal.*`, over a block's grid.

use std::path::Path;

use rusty_mux_core::{Progress, Selection};
use rusty_mux_proto::theme::{Colour, ThemeChange};
use rusty_mux_status::Records;
use rusty_term_core::{Cell, Grid, ShellPhase, StatusSignal, WIDE_TRAILER};

use crate::style::Style;

/// What `terminal.reset` feeds a terminal: CAN, which ends any sequence the
/// program left unfinished (a string, an ESC waiting for its next byte, a
/// cut CSI or UTF-8 character) without acting on it, then RIS. Without the
/// CAN, rusty_term_core reads the RIS's ESC as part of an unfinished string
/// or escape and never resets, while a VT500-style outer terminal would.
/// Clients built before CAN ended an OSC string take it into the string;
/// a reset is refused while one is attached.
pub(crate) const RESET: &[u8] = b"\x18\x1bc";

/// The OSC sequences that make `change`, each ended by ST: OSC 10, 11 and
/// 12 set the default foreground, background and cursor colours, 110, 111
/// and 112 put them back, and OSC 4 and 104 do the same for a palette
/// entry. None asks the terminal anything, so none is answered.
pub(crate) fn theme(change: &ThemeChange) -> Vec<u8> {
    let mut out = String::new();
    let dynamic = [
        (change.foreground, 10),
        (change.background, 11),
        (change.cursor, 12),
    ];
    for (colour, code) in dynamic {
        match colour {
            Some(Colour::Rgb(rgb)) => out.push_str(&format!("\x1b]{code};#{rgb:06x}\x1b\\")),
            Some(Colour::Default) => out.push_str(&format!("\x1b]1{code}\x1b\\")),
            None => {}
        }
    }
    for (index, colour) in &change.palette {
        match colour {
            Colour::Rgb(rgb) => out.push_str(&format!("\x1b]4;{index};#{rgb:06x}\x1b\\")),
            Colour::Default => out.push_str(&format!("\x1b]104;{index}\x1b\\")),
        }
    }
    out.into_bytes()
}

/// What `terminal.clear` feeds a terminal, as `clear` writes it: the
/// cursor home, the screen erased (ED 2), then the scrollback (ED 3).
/// Nothing asks the terminal anything, so nothing is answered.
pub(crate) const CLEAR: &[u8] = b"\x1b[H\x1b[2J\x1b[3J";

/// The mouse tracking DEC mode `base` turns on, as `terminal.modes` names
/// it.
fn mouse_tracking(base: usize) -> &'static str {
    match base {
        1000 => "click",
        1002 => "drag",
        1003 => "any",
        _ => "off",
    }
}

/// The mouse report encodings in `extended`, as `terminal.modes` names
/// them: `default` when none is on.
fn mouse_encoding(extended: u8) -> String {
    let names: Vec<&str> = [(1, "utf8"), (2, "sgr"), (4, "urxvt"), (8, "sgr-pixels")]
        .into_iter()
        .filter(|(bit, _)| extended & bit != 0)
        .map(|(_, name)| name)
        .collect();
    match names.is_empty() {
        true => "default".to_owned(),
        false => names.join(","),
    }
}

/// `terminal.modes`: the modes `grid`'s program set, one `NAME<TAB>VALUE`
/// line each.
pub(crate) fn modes(grid: &Grid) -> String {
    let flag = |on: bool| if on { "on" } else { "off" }.to_owned();
    let mouse = grid.mouse_modes;
    let lines = [
        ("app-cursor-keys", flag(grid.app_cursor_keys)),
        ("app-keypad", flag(grid.app_keypad)),
        ("bracketed-paste", flag(grid.bracketed_paste)),
        ("focus-reporting", flag(grid.focus_reporting)),
        ("mouse", mouse_tracking(mouse.base).to_owned()),
        ("mouse-encoding", mouse_encoding(mouse.extended)),
        ("alt-screen", flag(grid.in_alt_screen())),
        ("alt-scroll", flag(grid.alt_scroll)),
        ("insert", flag(grid.insert_mode())),
        ("cursor-visible", flag(grid.cursor_visible)),
        ("cursor-blink", flag(grid.cursor_blink)),
        ("report-color-scheme", flag(grid.report_color_scheme)),
        ("win32-input", flag(grid.win32_input)),
        ("kitty-keyboard", grid.kitty_keyboard_flags().to_string()),
    ];
    lines
        .iter()
        .map(|(name, value)| format!("{name}\t{value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `terminal.shell`: what the block's shell is and does, one
/// `NAME<TAB>VALUE` line each. `program` is what the block started and
/// `activity` comes from the process table (see [`crate::process::activity`]);
/// `phase` and `exit` come from the shell's OSC 133 marks in `grid`, and
/// are `unknown` until it sends one, as a shell without that integration
/// never does. The program is [`crate::process::escape`]d, so its path can
/// add neither a field nor a line.
pub(crate) fn shell(grid: &Grid, program: &str, activity: &str) -> String {
    let phase = match grid.shell_phase() {
        None => "unknown",
        Some(ShellPhase::Prompt) => "prompt",
        Some(ShellPhase::Running) => "running",
        Some(ShellPhase::Finished) => "finished",
    };
    let exit = grid
        .last_command_exit()
        .map_or_else(|| "unknown".to_owned(), |code| code.to_string());
    [
        ("program", crate::process::escape(program)),
        ("activity", activity.to_owned()),
        ("phase", phase.to_owned()),
        ("exit", exit),
    ]
    .iter()
    .map(|(name, value)| format!("{name}\t{value}"))
    .collect::<Vec<_>>()
    .join("\n")
}

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
/// cut short. A wrapped run is refused once it holds more than `2 * max`
/// cells, before its text is built: every cell of it but a wide glyph's
/// second prints, unless the run ends in blanks.
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
            // A soft-wrapped row is stored whole, at the width it had when
            // it wrapped: a history row can be narrower than the grid.
            run.append(&mut cells);
            if run.len() / 2 > max {
                return None;
            }
            continue;
        }
        run.append(&mut cells);
        let breaks = held + usize::from(lines > 0);
        // What the line may add; a blank one `trim` holds back adds nothing.
        let room = max.saturating_sub(text.len() + breaks);
        let line = line(&run, &grid.clusters, capture.vt, room)?;
        run.clear();
        if capture.trim && line.is_empty() {
            // Leading blanks are dropped, later ones held.
            held += usize::from(lines > 0);
            continue;
        }
        for _ in 0..breaks {
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
/// prints the same under any pen. `None` once it passes `max` bytes, before
/// it grows further.
fn line(cells: &[Cell], clusters: &[String], vt: bool, max: usize) -> Option<String> {
    let ends = |c: &Cell| match vt {
        true => !is_padding(c),
        false => !c.ch.is_whitespace() || c.cluster != 0,
    };
    let len = cells.iter().rposition(ends).map_or(0, |i| i + 1);
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
        if text.len() > max {
            return None;
        }
    }
    if style.is_some_and(|s| s != blank) {
        text.push_str("\x1b[0m");
    }
    if !vt {
        text.truncate(text.trim_end().len());
    }
    Some(text)
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

/// What applying a block's status signals did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Absorbed {
    /// Whether any record's state changed, the part worth announcing;
    /// progress and text alone are not.
    pub(crate) changed: bool,
    /// The ids of the records that ended, in id order. A record cleared
    /// and reported again in the same read did not end.
    pub(crate) removed: Vec<String>,
}

/// Applies the status signals the grid collected since the last call: OSC
/// 7501 reports, prompts and resets.
pub(crate) fn absorb_status(grid: &mut Grid, records: &mut Records) -> Absorbed {
    if grid.program_status.is_empty() {
        return Absorbed::default();
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
    let after = records.states();
    // Both are in id order, as `Records` keeps them.
    let removed = before
        .iter()
        .filter(|(id, ..)| after.binary_search_by(|(a, ..)| a.cmp(id)).is_err())
        .map(|(id, ..)| id.as_str().to_owned())
        .collect();
    Absorbed {
        changed: after != before,
        removed,
    }
}

/// The selections the block's program wrote (OSC 52) since the last call,
/// clipboard first; the text itself is let go.
pub(crate) fn take_clipboard(grid: &mut Grid) -> Vec<Selection> {
    let clipboard = grid.clipboard_set.take().map(|_| Selection::Clipboard);
    let primary = grid
        .clipboard_set_primary
        .take()
        .map(|_| Selection::Primary);
    clipboard.into_iter().chain(primary).collect()
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
    use rusty_term_core::AnsiParser;

    /// Every state a program can leave the parser in, as the bytes that
    /// leave it there: strings, an ESC inside a string (which starts its
    /// terminator), escapes waiting for a byte, a CSI and a UTF-8
    /// character cut short.
    pub(crate) const STUCK: &[&[u8]] = &[
        b"\x1b]0;stuck",
        b"\x1b]0;stuck\x1b",
        b"\x1bP1$r",
        b"\x1bP1$r\x1b",
        b"\x1b_Gstuck",
        b"\x1b_Gstuck\x1b",
        b"\x1b^stuck",
        b"\x1b^stuck\x1b",
        b"\x1bXstuck\x1b",
        b"\x1b",
        b"\x1b(",
        b"\x1b#",
        b"\x1b[3",
        b"\xe2\x94",
    ];

    #[test]
    fn a_reset_resets_whatever_the_program_left_unfinished() {
        for stuck in STUCK {
            let mut grid = Grid::new(20, 3);
            let mut parser = AnsiParser::new();
            parser.advance(&mut grid, b"\x1b[?1hBEFORE");
            parser.advance(&mut grid, stuck);
            parser.advance(&mut grid, RESET);
            let screen = format(&grid, Capture::default(), usize::MAX).unwrap_or_default();
            assert!(!screen.contains("BEFORE"), "{stuck:?}: {screen:?}");
            assert!(!grid.app_cursor_keys, "{stuck:?}");
            assert!(parser.ground(), "{stuck:?}");
            assert_eq!(grid.title, "", "{stuck:?}: the string was not acted on");
            assert!(parser.take_responses().is_empty(), "{stuck:?}");
        }
    }

    /// `terminal.modes` as a map from name to value.
    fn mode_map(grid: &Grid) -> Vec<(String, String)> {
        modes(grid)
            .lines()
            .map(|l| {
                let (name, value) = l.split_once('\t').unwrap_or((l, ""));
                (name.to_owned(), value.to_owned())
            })
            .collect()
    }

    fn mode(grid: &Grid, name: &str) -> String {
        let found = mode_map(grid).into_iter().find(|(n, _)| n == name);
        found.map(|(_, v)| v).unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn modes_start_as_a_fresh_terminal_sets_them() {
        let grid = Grid::new(20, 3);
        let names: Vec<String> = mode_map(&grid).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "app-cursor-keys",
                "app-keypad",
                "bracketed-paste",
                "focus-reporting",
                "mouse",
                "mouse-encoding",
                "alt-screen",
                "alt-scroll",
                "insert",
                "cursor-visible",
                "cursor-blink",
                "report-color-scheme",
                "win32-input",
                "kitty-keyboard",
            ]
        );
        assert_eq!(mode(&grid, "cursor-visible"), "on");
        assert_eq!(mode(&grid, "bracketed-paste"), "off");
        assert_eq!(mode(&grid, "mouse"), "off");
        assert_eq!(mode(&grid, "mouse-encoding"), "default");
        assert_eq!(mode(&grid, "kitty-keyboard"), "0");
    }

    #[test]
    fn modes_follow_what_the_program_sets() {
        let mut grid = Grid::new(20, 3);
        let mut parser = AnsiParser::new();
        parser.advance(
            &mut grid,
            b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1004h\x1b[?1002h\x1b[?1006h\x1b[?1016h\
              \x1b[?1049h\x1b[4h\x1b[?25l\x1b[>5u",
        );
        for (name, value) in [
            ("app-cursor-keys", "on"),
            ("app-keypad", "on"),
            ("bracketed-paste", "on"),
            ("focus-reporting", "on"),
            ("mouse", "drag"),
            ("mouse-encoding", "sgr,sgr-pixels"),
            ("alt-screen", "on"),
            ("insert", "on"),
            ("cursor-visible", "off"),
            ("kitty-keyboard", "5"),
        ] {
            assert_eq!(mode(&grid, name), value, "{name}");
        }
        parser.advance(&mut grid, b"\x1b[?1003h\x1b[?1002l\x1b[?1049l");
        assert_eq!(mode(&grid, "mouse"), "any");
        assert_eq!(mode(&grid, "alt-screen"), "off");
    }

    /// The value of `terminal.shell`'s line `name`.
    fn shell_line(grid: &Grid, name: &str) -> String {
        let text = shell(grid, "/bin/sh", "idle");
        let found = text
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{name}\t")));
        found.unwrap_or_default().to_owned()
    }

    #[test]
    fn shell_reports_unknown_marks_until_the_shell_sends_one() {
        let grid = Grid::new(20, 3);
        let text = shell(&grid, "/bin/sh", "idle");
        assert_eq!(
            text,
            "program\t/bin/sh\nactivity\tidle\nphase\tunknown\nexit\tunknown"
        );
    }

    #[test]
    fn shell_follows_osc_133_marks() {
        let mut grid = Grid::new(20, 3);
        let mut parser = AnsiParser::new();
        parser.advance(&mut grid, b"\x1b]133;A\x07$ ");
        assert_eq!(shell_line(&grid, "phase"), "prompt");
        parser.advance(&mut grid, b"\x1b]133;C\x07");
        assert_eq!(shell_line(&grid, "phase"), "running");
        parser.advance(&mut grid, b"\x1b]133;D;127\x07");
        assert_eq!(shell_line(&grid, "phase"), "finished");
        assert_eq!(shell_line(&grid, "exit"), "127");
    }

    #[test]
    fn shell_escapes_the_program() {
        let grid = Grid::new(20, 3);
        let text = shell(&grid, "/odd\tpath\nx", "busy");
        assert_eq!(text.lines().count(), 4, "{text:?}");
        assert!(text.starts_with("program\t/odd\\tpath\\nx\n"), "{text:?}");
    }

    #[test]
    fn a_clear_empties_the_screen_and_scrollback_and_keeps_modes() {
        let mut grid = Grid::new(20, 3);
        let mut parser = AnsiParser::new();
        parser.advance(&mut grid, b"\x1b]0;kept\x07\x1b[?2004h");
        for n in 0..10 {
            parser.advance(&mut grid, format!("LINE{n}\r\n").as_bytes());
        }
        let all = Capture {
            history: 1000,
            ..Capture::default()
        };
        let before = format(&grid, all, usize::MAX).unwrap_or_default();
        assert!(before.contains("LINE0"), "scrolled back: {before:?}");
        parser.advance(&mut grid, CLEAR);
        let screen = format(&grid, all, usize::MAX).unwrap_or_default();
        assert!(!screen.contains("LINE"), "{screen:?}");
        assert_eq!(grid.cursor, (0, 0));
        assert!(grid.bracketed_paste);
        assert_eq!(grid.title, "kept");
        assert!(parser.take_responses().is_empty());
    }

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
        let changed = |grid: &mut Grid, records: &mut Records| absorb_status(grid, records).changed;
        assert!(!changed(&mut grid, &mut records), "nothing collected");
        parser.advance(&mut grid, b"\x1b]7501;state=working:progress=10\x07");
        assert!(changed(&mut grid, &mut records));
        parser.advance(&mut grid, b"\x1b]7501;state=working:progress=90\x07");
        assert!(!changed(&mut grid, &mut records), "progress only");
        parser.advance(&mut grid, b"\x1b]7501;state=bogus\x07");
        assert!(!changed(&mut grid, &mut records), "ignored report");
        assert!(grid.program_status.is_empty(), "drained");
        assert_eq!(records.bodies(), ["state=working:progress=90"]);
    }

    #[test]
    fn prompts_and_resets_end_records() {
        let (mut grid, mut parser, mut records) = status_grid();
        parser.advance(&mut grid, b"\x1b]7501;state=blocked:kind=question\x07");
        parser.advance(&mut grid, b"\x1b]7501;state=done:id=build\x07");
        assert_eq!(absorb_status(&mut grid, &mut records), changed(&[]));
        parser.advance(&mut grid, b"\x1b]133;A\x07");
        assert_eq!(absorb_status(&mut grid, &mut records), changed(&[""]));
        assert_eq!(records.bodies(), ["state=done:id=build"]);
        parser.advance(&mut grid, b"\x1bc");
        assert_eq!(absorb_status(&mut grid, &mut records), changed(&["build"]));
        assert!(records.bodies().is_empty());
    }

    fn changed(removed: &[&str]) -> Absorbed {
        Absorbed {
            changed: true,
            removed: removed.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn a_clear_removes_the_record_and_those_beneath_it() {
        let (mut grid, mut parser, mut records) = status_grid();
        for id in ["a", "a/x", "ab", "b"] {
            let report = format!("\x1b]7501;state=working:id={id}\x07");
            parser.advance(&mut grid, report.as_bytes());
        }
        absorb_status(&mut grid, &mut records);
        parser.advance(&mut grid, b"\x1b]7501;state=clear:id=a\x07");
        assert_eq!(
            absorb_status(&mut grid, &mut records),
            changed(&["a", "a/x"])
        );
        // Cleared and reported again in one read: it never ended.
        parser.advance(
            &mut grid,
            b"\x1b]7501;state=clear:id=b\x07\x1b]7501;state=done:id=b\x07",
        );
        assert_eq!(absorb_status(&mut grid, &mut records), changed(&[]));
        parser.advance(&mut grid, b"\x1b]7501;state=clear:id=nothing\x07");
        assert_eq!(absorb_status(&mut grid, &mut records), Absorbed::default());
    }

    #[test]
    fn clipboard_writes_are_taken_once_each() {
        let mut grid = Grid::new(20, 2);
        let mut parser = rusty_term_core::AnsiParser::new();
        assert!(take_clipboard(&mut grid).is_empty());
        // "aGk=" is "hi"; a query and undecodable data are not writes.
        parser.advance(
            &mut grid,
            b"\x1b]52;p;aGk=\x07\x1b]52;c;aGk=\x07\x1b]52;;aGk=\x07",
        );
        parser.advance(&mut grid, b"\x1b]52;c;?\x07\x1b]52;p;!!\x07");
        assert_eq!(
            take_clipboard(&mut grid),
            [Selection::Clipboard, Selection::Primary]
        );
        assert!(take_clipboard(&mut grid).is_empty(), "taken");
    }

    #[test]
    fn a_report_after_an_overflowing_burst_still_lands() {
        let (mut grid, mut parser, mut records) = status_grid();
        let round = b"\x1b]7501;state=working:id=a\x07\x1b]133;A\x07";
        let burst = round.repeat(rusty_term_core::PROGRAM_STATUS_MAX * 2);
        parser.advance(&mut grid, &burst);
        parser.advance(&mut grid, b"\x1b]7501;state=done\x07");
        assert_eq!(absorb_status(&mut grid, &mut records), changed(&[]));
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
    fn vt_keeps_overline() {
        let grid = screen(4, 1, b"\x1b[53mo\x1b[0m");
        let vt = Capture {
            vt: true,
            ..capture(0)
        };
        let text = format(&grid, vt, usize::MAX).expect("within the limit");
        assert_eq!(text, "\x1b[0;53mo\x1b[0m");
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
