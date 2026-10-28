//! The attach client for rusty_mux. It shows a block's window as the
//! server lays it out: one connection per visible block, each block's
//! output parsed into a local grid. A window showing one block passes its
//! raw output straight to the local terminal, which keeps every mode the
//! program sets; one showing several is composed into one screen
//! ([`screen`]) and redrawn where it changed. [`keys`] turns the keyboard
//! into input and prefix commands.

pub mod keys;
pub mod screen;

use std::fmt::Write as _;
use std::io::{self, Read, Write};

use rusty_mux_proto::{Message, decode, encode};
use rusty_term_core::{Grid, WIDE_TRAILER};

/// Written on every exit after raw output stops: leaves the alternate
/// screen, soft-resets (DECSTR: pen, cursor visibility and keys, margins,
/// charsets), restores the numeric keypad and turns off bracketed paste,
/// mouse and focus reporting and win32-input-mode. A block's program may
/// have set any of them on this terminal through the passthrough.
pub const RESTORE: &[u8] = b"\x1b[?1049l\x1b[!p\x1b[?25h\x1b>\x1b[?2004l\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?9001l\x1b[0m";

/// The longest unfinished mode set [`NoWin32Input`] holds back; past it
/// the bytes pass as they are, as no real program sends one so long.
const HOLD_MAX: usize = 64;

/// Drops win32-input-mode (DEC `?9001`) from the mode sets in a block's
/// raw output, so the local terminal keeps sending plain VT keys: the
/// client reads the keyboard itself, for its Ctrl-] commands. A Windows
/// server's pseudo console asks for the mode at start and takes plain VT
/// input as well.
///
/// Chunk boundaries are not sequence boundaries: a mode set split across
/// chunks is held back until it completes, and every other byte passes
/// unchanged and in order.
#[derive(Debug, Default)]
pub struct NoWin32Input {
    held: Vec<u8>,
}

impl NoWin32Input {
    /// The next chunk of a block's raw output, as written to the local
    /// terminal.
    pub fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        let input = [std::mem::take(&mut self.held).as_slice(), data].concat();
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while let Some(at) = input[i..].iter().position(|&b| b == 0x1b) {
            let start = i + at;
            out.extend_from_slice(&input[i..start]);
            match mode_set(&input[start..]) {
                Scan::Set(len) => {
                    out.extend(without_win32(&input[start..start + len]));
                    i = start + len;
                }
                Scan::Unfinished if input.len() - start <= HOLD_MAX => {
                    self.held = input[start..].to_vec();
                    return out;
                }
                Scan::Unfinished | Scan::Other => {
                    out.push(0x1b);
                    i = start + 1;
                }
            }
        }
        out.extend_from_slice(&input[i..]);
        out
    }
}

/// What starts at an ESC.
enum Scan {
    /// A DEC private mode set, `ESC [ ? params h`, this many bytes long.
    Set(usize),
    /// What could still become one.
    Unfinished,
    Other,
}

fn mode_set(seq: &[u8]) -> Scan {
    for (i, &b) in seq.iter().enumerate().skip(1) {
        match (i, b) {
            (1, b'[') | (2, b'?') | (3.., b'0'..=b'9' | b';') => {}
            (3.., b'h') => return Scan::Set(i + 1),
            _ => return Scan::Other,
        }
    }
    Scan::Unfinished
}

/// A whole mode set `seq` without 9001: unchanged when it lacks it,
/// nothing when 9001 was all it set.
fn without_win32(seq: &[u8]) -> Vec<u8> {
    let params = &seq[3..seq.len() - 1];
    let is_win32 = |p: &&[u8]| {
        std::str::from_utf8(p)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            == Some(9001)
    };
    if !params.split(|&b| b == b';').any(|p| is_win32(&p)) {
        return seq.to_vec();
    }
    let kept: Vec<&[u8]> = params
        .split(|&b| b == b';')
        .filter(|p| !is_win32(p))
        .collect();
    if kept.iter().all(|p| p.is_empty()) {
        return Vec::new();
    }
    [b"\x1b[?", kept.join(&b';').as_slice(), b"h"].concat()
}

/// Repaints the local terminal from a restored grid: every visible row
/// with its colours, then the cursor. Attributes other than colour are not
/// painted; the next output redraws whatever the program redraws.
pub fn paint(grid: &Grid) -> Vec<u8> {
    let frame = grid.snapshot_viewport();
    // `write!` to a String cannot fail, so its results are ignored below.
    let mut out = String::from("\x1b[0m\x1b[2J");
    for (y, cells) in &frame.rows {
        let _ = write!(out, "\x1b[{};1H", y + 1);
        let mut style = None;
        for cell in cells {
            if cell.flags & WIDE_TRAILER != 0 {
                continue;
            }
            if style != Some((cell.fg, cell.bg)) {
                style = Some((cell.fg, cell.bg));
                let (fg, bg) = (rgb(cell.fg), rgb(cell.bg));
                let _ = write!(
                    out,
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                    fg.0, fg.1, fg.2, bg.0, bg.1, bg.2
                );
            }
            out.push(cell.ch);
            if let Some(rest) = usize::from(cell.cluster)
                .checked_sub(1)
                .and_then(|i| frame.clusters.get(i))
            {
                out.push_str(rest);
            }
        }
    }
    let (x, y) = frame.cursor;
    let _ = write!(out, "\x1b[0m\x1b[{};{}H", y + 1, x + 1);
    out.into_bytes()
}

/// Hands the local terminal to `grid`'s program: its screen (main or
/// alternate) and scroll region, its cells and cursor, then its [`modes`].
/// Used when a block's raw output starts passing through, since the
/// program will not set its modes again.
pub fn pass(grid: &Grid) -> Vec<u8> {
    let screen: &[u8] = match grid.in_alt_screen() {
        true => b"\x1b[?1049h",
        false => b"\x1b[?1049l",
    };
    // DECSTBM homes the cursor, so it goes before the paint places it.
    let region = format!("\x1b[{};{}r", grid.scroll_top + 1, grid.scroll_bottom + 1);
    [screen, region.as_bytes(), &paint(grid), &modes(grid, true)].concat()
}

/// The input modes `grid`'s program set, each as a sequence that sets it on
/// the local terminal or clears it: cursor and keypad keys, bracketed
/// paste and alternate scroll; win32 input stays off (see [`NoWin32Input`]).
/// With `whole` the block has the
/// whole terminal, so its mouse and focus reports and its cursor's
/// visibility go too; otherwise those are cleared, as screen coordinates
/// and focus belong to the composed screen.
pub fn modes(grid: &Grid, whole: bool) -> Vec<u8> {
    let mut out = String::new();
    let mut dec = |mode: usize, on: bool| {
        let _ = write!(out, "\x1b[?{mode}{}", if on { 'h' } else { 'l' });
    };
    dec(1, grid.app_cursor_keys);
    dec(2004, grid.bracketed_paste);
    dec(1007, grid.alt_scroll);
    dec(9001, false);
    dec(1004, whole && grid.focus_reporting);
    let mouse = grid.mouse_modes;
    for mode in [1000, 1002, 1003] {
        dec(mode, whole && mouse.base == mode);
    }
    for (bit, mode) in [(1, 1005), (2, 1006), (4, 1015), (8, 1016)] {
        dec(mode, whole && mouse.extended & bit != 0);
    }
    if whole {
        dec(25, grid.cursor_visible);
    }
    out.push_str(if grid.app_keypad { "\x1b=" } else { "\x1b>" });
    out.into_bytes()
}

fn rgb(color: u32) -> (u8, u8, u8) {
    ((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// Blocking frame I/O over any byte stream.
pub struct Conn<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Conn<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    pub fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        self.stream.write_all(&frame)
    }

    /// The next message; `UnexpectedEof` when the server closes.
    pub fn recv(&mut self) -> io::Result<Message> {
        let mut chunk = [0u8; 65536];
        loop {
            let decoded =
                decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if let Some((message, used)) = decoded {
                self.buf.drain(..used);
                return Ok(message);
            }
            let n = self.stream.read(&mut chunk)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_mux_core::BlockId;
    use rusty_term_core::AnsiParser;
    use rusty_uuid::Uuid;

    #[test]
    fn modes_follow_what_the_program_set() {
        let mut grid = Grid::new(20, 3);
        let plain = String::from_utf8(modes(&grid, true)).unwrap();
        assert!(plain.contains("\x1b[?1l") && plain.contains("\x1b[?2004l"));
        assert!(plain.contains("\x1b[?25h") && plain.ends_with("\x1b>"));
        AnsiParser::new().advance(
            &mut grid,
            b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?25l",
        );
        let whole = String::from_utf8(modes(&grid, true)).unwrap();
        for set in [
            "?1h", "?2004h", "?1002h", "?1006h", "?1004h", "?25l", "?1000l",
        ] {
            assert!(whole.contains(&format!("\x1b[{set}")), "{set}: {whole:?}");
        }
        assert!(whole.ends_with("\x1b="));
        // Composed: keys as set, but no mouse or focus reports.
        let part = String::from_utf8(modes(&grid, false)).unwrap();
        for set in ["?1h", "?2004h", "?1002l", "?1006l", "?1004l"] {
            assert!(part.contains(&format!("\x1b[{set}")), "{set}: {part:?}");
        }
        assert!(!part.contains("?25"));
    }

    /// Feeds `chunks` through one filter, joined.
    fn filtered(chunks: &[&[u8]]) -> Vec<u8> {
        let mut filter = NoWin32Input::default();
        chunks.iter().flat_map(|c| filter.filter(c)).collect()
    }

    #[test]
    fn win32_input_mode_never_reaches_the_local_terminal() {
        // What a Windows pseudo console writes when it starts.
        assert_eq!(filtered(&[b"\x1b[?9001h\x1b[?1004hhi"]), b"\x1b[?1004hhi");
        assert_eq!(filtered(&[b"\x1b[?1;9001;2004h"]), b"\x1b[?1;2004h");
        assert_eq!(filtered(&[b"\x1b[?09001h\x1b[?9001l"]), b"\x1b[?9001l");
        let modes = String::from_utf8(modes(&Grid::new(20, 3), true)).unwrap();
        assert!(modes.contains("\x1b[?9001l"), "{modes:?}");
        assert!(String::from_utf8_lossy(RESTORE).contains("\x1b[?9001l"));
    }

    #[test]
    fn the_win32_filter_holds_a_mode_set_split_across_chunks() {
        let mut filter = NoWin32Input::default();
        assert_eq!(filter.filter(b"a\x1b[?90"), b"a");
        assert_eq!(filter.filter(b"01"), b"");
        assert_eq!(filter.filter(b"hb"), b"b");
        // Every split of a stream mixing a mode request with other
        // sequences and text gives the same bytes as the whole.
        let stream =
            "\x1b[?9001h\x1b[31mré\x1b]0;t\x07\x1bP1$r\x1b\\\x1b[?25;9001hx\x1b[?2004h".as_bytes();
        let whole = filtered(&[stream]);
        assert_eq!(
            whole,
            "\x1b[31mré\x1b]0;t\x07\x1bP1$r\x1b\\\x1b[?25hx\x1b[?2004h".as_bytes()
        );
        for at in 0..=stream.len() {
            for to in at..=stream.len() {
                let (a, rest) = stream.split_at(at);
                let (b, c) = rest.split_at(to - at);
                assert_eq!(filtered(&[a, b, c]), whole, "split at {at}, {to}");
            }
        }
    }

    #[test]
    fn the_win32_filter_passes_other_sequences_untouched() {
        let stream: &[u8] = b"\x1b[?1\x1b[?2004l\x1b[?;h\x1b[?12$p\x1b";
        assert_eq!(filtered(&[stream]), &stream[..stream.len() - 1]);
        // A run too long to be a real mode set is not held for ever.
        let long = [b"\x1b[?".as_slice(), &[b'1'; HOLD_MAX]].concat();
        assert_eq!(filtered(&[&long]), long);
    }

    #[test]
    fn pass_restores_the_screen_and_region_before_painting() {
        let mut grid = Grid::new(20, 5);
        AnsiParser::new().advance(&mut grid, b"\x1b[?1049h\x1b[2;4r\x1b[?1h");
        let passed = String::from_utf8(pass(&grid)).unwrap();
        assert!(
            passed.starts_with("\x1b[?1049h\x1b[2;4r\x1b[0m\x1b[2J"),
            "{passed:?}"
        );
        assert!(passed.contains("\x1b[?1h"));
    }

    #[test]
    fn paint_draws_text_and_places_the_cursor() {
        let mut grid = Grid::new(20, 3);
        AnsiParser::new().advance(&mut grid, b"hello\r\n\x1b[31mred\x1b[0m!");
        let painted = String::from_utf8(paint(&grid)).unwrap();
        assert!(painted.starts_with("\x1b[0m\x1b[2J"));
        assert!(painted.contains("hello"), "{painted:?}");
        // The red run and the default-coloured "!" are separate styles.
        assert!(
            painted.contains("38;2;128;0;0;48;2;0;0;0mred"),
            "{painted:?}"
        );
        assert!(painted.ends_with("\x1b[0m\x1b[2;5H"), "{painted:?}");
    }

    #[test]
    fn paint_skips_wide_trailers() {
        let mut grid = Grid::new(10, 1);
        AnsiParser::new().advance(&mut grid, "日x".as_bytes());
        let painted = String::from_utf8(paint(&grid)).unwrap();
        assert!(painted.contains("日x"), "{painted:?}");
    }

    #[test]
    fn conn_reassembles_split_frames() {
        let block = BlockId::from_uuid(Uuid::nil());
        let message = Message::Output {
            block,
            data: b"abc".to_vec(),
        };
        let mut wire = encode(&message).unwrap();
        wire.extend(encode(&Message::Ready { block }).unwrap());
        let mut conn = Conn::new(io::Cursor::new(wire));
        assert_eq!(conn.recv().unwrap(), message);
        assert_eq!(conn.recv().unwrap(), Message::Ready { block });
        assert_eq!(
            conn.recv().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
