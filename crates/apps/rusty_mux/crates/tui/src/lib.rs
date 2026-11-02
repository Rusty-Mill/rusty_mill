//! The attach client for rusty_mux. It shows a block's window as the
//! server lays it out: one connection per visible block, each block's
//! output parsed into a local grid. A window showing one block passes its
//! raw output straight to the local terminal, which keeps every mode the
//! program sets; one showing several is composed into one screen
//! ([`screen`]) and redrawn where it changed. [`keys`] turns the keyboard
//! into input and prefix commands. [`predict`] guesses typed keys' echo
//! ahead of a slow server. [`ssh`] reaches a server on another
//! machine through an OpenSSH socket forward, and [`udp`] moves it onto a
//! UDP association when the server offers one.

pub mod keys;
pub mod predict;
pub mod screen;
#[cfg(unix)]
pub mod ssh;
#[cfg(unix)]
pub mod udp;

use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::path::Path;

use rusty_mux_proto::{Message, decode, encode};
use rusty_term_core::{Grid, WIDE_TRAILER};

/// Written on every exit after raw output stops: leaves the alternate
/// screen, soft-resets (DECSTR: pen, cursor visibility and keys, margins,
/// charsets), restores the numeric keypad and turns off bracketed paste,
/// mouse and focus reporting and win32-input-mode, and puts the terminal's
/// own cursor colour back (OSC 112). A block's program may have set any of
/// them on this terminal through the passthrough.
pub const RESTORE: &[u8] = b"\x1b[?1049l\x1b[!p\x1b[?25h\x1b>\x1b[?2004l\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?9001l\x1b[0m\
\x1b]112\x1b\\";

/// The longest unfinished mode set [`NoWin32Input`] holds back. Past it
/// the bytes pass as they are, so a set split across reads after this
/// many bytes keeps its 9001; no real program sends one so long, and a
/// hold without a bound would let a block's output grow it for ever.
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

/// A C0 control a terminal runs in the middle of a sequence, which goes
/// on after it; CAN and SUB cancel the sequence and ESC starts another.
fn runs_inside(b: u8) -> bool {
    b < 0x20 && !matches!(b, 0x18 | 0x1a | 0x1b)
}

fn mode_set(seq: &[u8]) -> Scan {
    // How far into `ESC [ ? params h`, the C0 controls inside it aside.
    let mut at = 1;
    for (i, &b) in seq.iter().enumerate().skip(1) {
        if runs_inside(b) {
            continue;
        }
        match (at, b) {
            (1, b'[') | (2, b'?') | (3.., b'0'..=b'9' | b';') => at += 1,
            (3.., b'h') => return Scan::Set(i + 1),
            _ => return Scan::Other,
        }
    }
    Scan::Unfinished
}

/// A whole mode set `seq` without 9001: unchanged when it lacks it,
/// nothing when 9001 was all it set. The C0 controls inside it go first,
/// as the terminal runs them before the set completes.
fn without_win32(seq: &[u8]) -> Vec<u8> {
    let (controls, set): (Vec<u8>, Vec<u8>) = seq.iter().partition(|&&b| runs_inside(b));
    let params = &set[3..set.len() - 1];
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
        return controls;
    }
    [&controls, b"\x1b[?".as_slice(), &kept.join(&b';'), b"h"].concat()
}

/// Creates the key log at `path`, which must not exist yet (a symlink
/// included). It records everything typed, so it is the owner's alone
/// before any byte goes in: mode 0600 on Unix, and on Windows an
/// owner-only DACL set while the open handle shares no access, failing
/// closed (the empty file removed) if it cannot be set. As `rusty_mux_ctl`
/// creates its private agent files.
pub fn create_private(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 0);
    let file = options.open(path)?;
    #[cfg(windows)]
    if let Err(e) = platform_windows::fs::restrict_to_owner(path) {
        drop(file);
        // The DACL's error is the one to report; the empty file is ours.
        let _ = std::fs::remove_file(path);
        return Err(io::Error::other(format!("restricting access: {e}")));
    }
    Ok(file)
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
/// alternate) and scroll region, its cells and cursor, then its [`modes`]
/// and the cursor colour it chose, if any. Used when a block's raw output
/// starts passing through, since the program will not set them again.
pub fn pass(grid: &Grid) -> Vec<u8> {
    let alt: &[u8] = match grid.in_alt_screen() {
        true => b"\x1b[?1049h",
        false => b"\x1b[?1049l",
    };
    // DECSTBM homes the cursor, so it goes before the paint places it.
    let region = format!("\x1b[{};{}r", grid.scroll_top + 1, grid.scroll_bottom + 1);
    let colour = screen::chosen_cursor(grid).map(|c| screen::cursor_colour(Some(c)));
    let colour = colour.unwrap_or_default();
    [
        alt,
        region.as_bytes(),
        &paint(grid),
        &modes(grid, true),
        colour.as_bytes(),
    ]
    .concat()
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

    /// The stream, to set its options.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    pub fn send(&mut self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        self.stream.write_all(&frame)
    }

    /// The next message; `UnexpectedEof` when the server closes.
    pub fn recv(&mut self) -> io::Result<Message> {
        loop {
            if let Some(message) = self.try_recv()? {
                return Ok(message);
            }
        }
    }

    /// The next message if one is buffered or a single read completes
    /// one, else `None`, so a caller can check a deadline between reads
    /// however the bytes trickle in. What a read brings is kept, also when
    /// it fails (a read timeout, say), for the next call.
    pub fn try_recv(&mut self) -> io::Result<Option<Message>> {
        if let Some(message) = self.decoded()? {
            return Ok(Some(message));
        }
        let mut chunk = [0u8; 65536];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.buf.extend_from_slice(&chunk[..n]);
        self.decoded()
    }

    /// Takes the buffered message, if a whole one is in.
    fn decoded(&mut self) -> io::Result<Option<Message>> {
        let decoded =
            decode(&self.buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(decoded.map(|(message, used)| {
            self.buf.drain(..used);
            message
        }))
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

    /// Each read is its own step: a frame that arrives in parts is
    /// `None` until its last part, and nothing read is lost.
    #[cfg(unix)]
    #[test]
    fn try_recv_returns_after_each_read() {
        let (mut near, far) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut conn = Conn::new(far);
        let frame = rusty_mux_proto::encode(&Message::Sync { req: 7 }).unwrap();
        let (head, tail) = frame.split_at(3);
        near.write_all(head).unwrap();
        assert!(conn.try_recv().unwrap().is_none());
        near.write_all(tail).unwrap();
        assert!(matches!(
            conn.try_recv().unwrap(),
            Some(Message::Sync { req: 7 })
        ));
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
        assert!(String::from_utf8_lossy(RESTORE).ends_with("\x1b]112\x1b\\"));
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
    fn the_win32_filter_sees_through_controls_inside_a_mode_set() {
        // A terminal runs NUL, BEL and CR mid-sequence and goes on with it.
        assert_eq!(filtered(&[b"\x1b[?90\x0001\x07h"]), b"\x00\x07");
        assert_eq!(filtered(&[b"\x1b\r[?25;9001h"]), b"\r\x1b[?25h");
        assert_eq!(filtered(&[b"\x1b[?900", b"\x001h"]), b"\x00");
        // CAN cancels it, so what follows is text.
        let cancelled: &[u8] = b"\x1b[?90\x1801h";
        assert_eq!(filtered(&[cancelled]), cancelled);
    }

    #[test]
    fn the_win32_filter_passes_other_sequences_untouched() {
        let stream: &[u8] = b"\x1b[?1\x1b[?2004l\x1b[?;h\x1b[?12$p\x1b";
        assert_eq!(filtered(&[stream]), &stream[..stream.len() - 1]);
        // A run too long to be a real mode set is not held for ever.
        let long = [b"\x1b[?".as_slice(), &[b'1'; HOLD_MAX]].concat();
        assert_eq!(filtered(&[&long]), long);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rusty-mux-tui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn a_private_file_is_new() {
        let dir = scratch("new");
        let path = dir.join("keys");
        std::fs::write(&path, "kept").unwrap();
        let refused = create_private(&path).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_the_owners_alone_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("unix");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let path = dir.join("keys");
        drop(create_private(&path).unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Whatever the directory grants and passes down, the file's DACL is
    /// protected with one entry, full control for the current user, by
    /// the time the handle is back. Checked by SID with `Get-Acl`.
    #[cfg(windows)]
    #[test]
    fn a_private_file_is_the_owners_alone_on_windows() {
        const DACL: &str = "$acl = Get-Acl -LiteralPath $env:ACL_PATH; \
            $acl.AreAccessRulesProtected; \
            $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) | \
            ForEach-Object { '{0} {1} {2} {3}' -f $_.IdentityReference.Value, \
            $_.AccessControlType, [int]$_.FileSystemRights, $_.IsInherited }";
        let powershell = |script: &str, path: &Path| {
            // Windows PowerShell 5.1, run from PowerShell 7, would inherit
            // 7's module path and fail to load `Get-Acl`.
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env_remove("PSModulePath")
                .env("ACL_PATH", path)
                .output()
                .expect("powershell");
            assert!(out.status.success(), "{out:?}");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let dacl = |path: &Path| -> (bool, Vec<String>) {
            let text = powershell(DACL, path);
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let protected = lines.next() == Some("True");
            (protected, lines.map(str::to_owned).collect())
        };
        let dir = scratch("windows");
        // Everyone (S-1-1-0) may read the directory and what it holds.
        let grant = "$acl = Get-Acl -LiteralPath $env:ACL_PATH; \
            $rule = New-Object Security.AccessControl.FileSystemAccessRule(\
            (New-Object Security.Principal.SecurityIdentifier('S-1-1-0')), 'Read', \
            'ContainerInherit,ObjectInherit', 'None', 'Allow'); \
            $acl.AddAccessRule($rule); Set-Acl -LiteralPath $env:ACL_PATH -AclObject $acl";
        powershell(grant, &dir);
        let (_, parent) = dacl(&dir);
        assert!(
            parent.iter().any(|e| e.starts_with("S-1-1-0 Allow")),
            "{parent:?}"
        );
        let me = powershell(
            "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
            &dir,
        );
        let me = me.trim();
        assert!(me.starts_with("S-1-"), "the current user's SID: {me}");

        let path = dir.join("keys");
        let mut file = create_private(&path).unwrap();
        file.write_all(b"1d 64\n").unwrap();
        drop(file);
        let (protected, entries) = dacl(&path);
        let ok = protected && entries == [format!("{me} Allow 2032127 False")];
        assert!(ok, "protected={protected} {entries:?}");
        std::fs::remove_dir_all(&dir).unwrap();
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
        assert!(!passed.contains("\x1b]1"), "no cursor colour: {passed:?}");
        AnsiParser::new().advance(&mut grid, b"\x1b]12;#ff8000\x07");
        let passed = String::from_utf8(pass(&grid)).unwrap();
        assert!(passed.ends_with("\x1b]12;#ff8000\x1b\\"), "{passed:?}");
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
