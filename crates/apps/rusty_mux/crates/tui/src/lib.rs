//! Throwaway Phase 1 client for rusty_mux. It attaches the local terminal
//! to one block: the attach checkpoint's grid is painted once, then the
//! block's raw output goes straight to the local terminal, which does the
//! parsing. Phase 2 replaces it with real clients.

use std::fmt::Write as _;
use std::io::{self, Read, Write};

use rusty_mux_proto::{Message, decode, encode};
use rusty_term_core::{Grid, WIDE_TRAILER};

/// Written on every exit after raw output stops: leaves the alternate
/// screen, soft-resets (DECSTR: pen, cursor visibility and keys, margins,
/// charsets), restores the numeric keypad and turns off bracketed paste,
/// mouse and focus reporting. A block's program may have set any of them on
/// this terminal through the passthrough.
pub const RESTORE: &[u8] = b"\x1b[?1049l\x1b[!p\x1b[?25h\x1b>\x1b[?2004l\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[0m";

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
