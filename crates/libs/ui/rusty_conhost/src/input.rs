//! Terminal input (design §5.5): the VT and win32-input-mode decoder,
//! the key records it produces, and the INPUT_RECORD queue.
//!
//! The terminal's bytes are split into [`Piece`]s, each keeping its raw
//! bytes. How a piece becomes records depends on the input mode, and is
//! the console's call: with VIRTUAL_TERMINAL_INPUT the raw bytes pass
//! through as character records ([`text_events`]), otherwise keys are
//! synthesised ([`key_events`]). A win32-input-mode record is exact in
//! both modes, and turned back into VT ([`vt_for`]) for a VT client.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/terminal/parser/InputStateMachineEngine.cpp`,
//! `src/host/inputBuffer.cpp`, `src/terminal/input/terminalInput.cpp`;
//! see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use std::collections::VecDeque;

use crate::codepage::{self, CodePage, Codec};

/// Records the queue holds; past it the oldest is dropped (§6.1).
pub(crate) const MAX_RECORDS: usize = 4096;
/// Bytes of an unfinished escape sequence kept for the next input; a
/// longer one is given up as unknown.
const MAX_PENDING: usize = 4096;
/// Bytes in one INPUT_RECORD.
pub(crate) const RECORD_LEN: usize = 20;

/// Virtual-key codes.
pub(crate) mod vk {
    pub(crate) const CANCEL: u16 = 0x03;
    pub(crate) const BACK: u16 = 0x08;
    pub(crate) const TAB: u16 = 0x09;
    #[cfg(any(test, feature = "fuzz"))]
    pub(crate) const RETURN: u16 = 0x0D;
    pub(crate) const SHIFT: u16 = 0x10;
    pub(crate) const CONTROL: u16 = 0x11;
    pub(crate) const MENU: u16 = 0x12;
    pub(crate) const ESCAPE: u16 = 0x1B;
    pub(crate) const PRIOR: u16 = 0x21;
    pub(crate) const NEXT: u16 = 0x22;
    pub(crate) const END: u16 = 0x23;
    pub(crate) const HOME: u16 = 0x24;
    pub(crate) const LEFT: u16 = 0x25;
    pub(crate) const UP: u16 = 0x26;
    pub(crate) const RIGHT: u16 = 0x27;
    pub(crate) const DOWN: u16 = 0x28;
    pub(crate) const INSERT: u16 = 0x2D;
    pub(crate) const DELETE: u16 = 0x2E;
    pub(crate) const C: u16 = 0x43;
    pub(crate) const LWIN: u16 = 0x5B;
    pub(crate) const RWIN: u16 = 0x5C;
    pub(crate) const F1: u16 = 0x70;
    pub(crate) const F12: u16 = 0x7B;
    pub(crate) const PACKET: u16 = 0xE7;
}

/// `dwControlKeyState` bits.
pub(crate) mod state {
    pub(crate) const RIGHT_ALT: u32 = 0x01;
    pub(crate) const LEFT_ALT: u32 = 0x02;
    pub(crate) const RIGHT_CTRL: u32 = 0x04;
    pub(crate) const LEFT_CTRL: u32 = 0x08;
    pub(crate) const SHIFT: u32 = 0x10;
    pub(crate) const ENHANCED_KEY: u32 = 0x100;
    pub(crate) const ALT: u32 = LEFT_ALT | RIGHT_ALT;
    pub(crate) const CTRL: u32 = LEFT_CTRL | RIGHT_CTRL;
}

/// The VK and `dwControlKeyState` that type a character on the active
/// keyboard layout (`condrv::key_for` on Windows, a US table in tests).
pub(crate) type KeyFor = fn(char) -> Option<(u16, u32)>;

/// `KEY_EVENT_RECORD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub down: bool,
    pub repeat: u16,
    pub vk: u16,
    pub scan: u16,
    /// A UTF-16 unit.
    pub ch: u16,
    pub state: u32,
}

impl KeyEvent {
    /// A key-down that only carries a character, as raw input does.
    fn text(ch: u16) -> KeyEvent {
        KeyEvent {
            down: true,
            repeat: 1,
            vk: 0,
            scan: 0,
            ch,
            state: 0,
        }
    }

    fn key(down: bool, vk: u16, ch: u16, state: u32) -> KeyEvent {
        KeyEvent {
            down,
            repeat: 1,
            vk,
            scan: 0,
            ch,
            state,
        }
    }
}

/// One INPUT_RECORD the console stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Record {
    Key(KeyEvent),
    Focus(bool),
    /// WINDOW_BUFFER_SIZE_EVENT: the buffer's new size.
    BufferSize {
        cols: u16,
        rows: u16,
    },
    /// Any other record a client wrote (mouse, menu), kept as written.
    Other {
        event_type: u16,
        event: [u8; 16],
    },
}

impl Record {
    /// The record as `INPUT_RECORD`: `EventType`, padding, then the event.
    pub(crate) fn encode(&self) -> [u8; RECORD_LEN] {
        let mut b = [0u8; RECORD_LEN];
        match self {
            Record::Key(k) => {
                b[0..2].copy_from_slice(&KEY_EVENT.to_le_bytes());
                b[4..8].copy_from_slice(&u32::from(k.down).to_le_bytes());
                b[8..10].copy_from_slice(&k.repeat.to_le_bytes());
                b[10..12].copy_from_slice(&k.vk.to_le_bytes());
                b[12..14].copy_from_slice(&k.scan.to_le_bytes());
                b[14..16].copy_from_slice(&k.ch.to_le_bytes());
                b[16..20].copy_from_slice(&k.state.to_le_bytes());
            }
            Record::Focus(on) => {
                b[0..2].copy_from_slice(&0x10u16.to_le_bytes());
                b[4..8].copy_from_slice(&u32::from(*on).to_le_bytes());
            }
            Record::BufferSize { cols, rows } => {
                b[0..2].copy_from_slice(&BUFFER_SIZE_EVENT.to_le_bytes());
                b[4..6].copy_from_slice(&cols.to_le_bytes());
                b[6..8].copy_from_slice(&rows.to_le_bytes());
            }
            Record::Other { event_type, event } => {
                b[0..2].copy_from_slice(&event_type.to_le_bytes());
                b[4..].copy_from_slice(event);
            }
        }
        b
    }

    /// An `INPUT_RECORD` a client wrote. Key and buffer-size events are
    /// typed, since reads and coalescing look into them; the rest stay
    /// as written.
    pub(crate) fn decode(b: &[u8; RECORD_LEN]) -> Record {
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        match u16_at(0) {
            KEY_EVENT => Record::Key(KeyEvent {
                down: u32_at(4) != 0,
                repeat: u16_at(8),
                vk: u16_at(10),
                scan: u16_at(12),
                ch: u16_at(14),
                state: u32_at(16),
            }),
            BUFFER_SIZE_EVENT => Record::BufferSize {
                cols: u16_at(4),
                rows: u16_at(6),
            },
            event_type => {
                let mut event = [0u8; 16];
                event.copy_from_slice(&b[4..]);
                Record::Other { event_type, event }
            }
        }
    }
}

/// `INPUT_RECORD.EventType` values the host looks into.
const KEY_EVENT: u16 = 0x1;
const MOUSE_EVENT: u16 = 0x2;
const BUFFER_SIZE_EVENT: u16 = 0x4;
/// `MOUSE_EVENT_RECORD.dwEventFlags`, at byte 12 of the event.
const MOUSE_MOVED: u32 = 0x1;
/// `dwControlKeyState`: an IME conversion, which coalesces whatever the
/// scan code.
const NLS_IME_CONVERSION: u32 = 0x0080_0000;

/// conhost `WriteConsoleInputAImpl`: each key record's character byte
/// (`uChar.AsciiChar`) in the input code page becomes one record per
/// UTF-16 unit. A DBCS lead byte joins the next record's byte; one that
/// ends the write waits in `lead` for the next write, and one followed
/// by a non-key record is dropped. Other records pass as they are.
pub(crate) fn ansi_records(
    records: &[Record],
    lead: &mut Option<KeyEvent>,
    cp: CodePage,
    codec: &Codec,
) -> Vec<Record> {
    if records.is_empty() {
        return Vec::new();
    }
    let byte = |k: &KeyEvent| k.ch.to_le_bytes()[0];
    let widen = |k: KeyEvent, narrow: &[u8], out: &mut Vec<Record>| {
        let text = codepage::decode(codec, cp.cp, narrow);
        out.extend(
            text.encode_utf16()
                .take(2)
                .map(|ch| Record::Key(KeyEvent { ch, ..k })),
        );
    };
    let mut out = Vec::with_capacity(records.len());
    let mut it = records.iter();
    if let (Some(first), Some(Record::Key(trail))) = (lead.take(), records.first()) {
        widen(first, &[byte(&first), byte(trail)], &mut out);
        it.next();
    }
    while let Some(r) = it.next() {
        let Record::Key(k) = r else {
            out.push(*r);
            continue;
        };
        if !cp.is_lead(byte(k)) {
            widen(*k, &[byte(k)], &mut out);
            continue;
        }
        match it.next() {
            None => *lead = Some(*k),
            Some(Record::Key(trail)) => widen(*k, &[byte(k), byte(trail)], &mut out),
            Some(_) => {}
        }
    }
    out
}

/// What a run of terminal input means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    /// Printable text.
    Text(String),
    /// A C0 control or DEL; ESC alone is the Escape key.
    Control(char),
    /// ESC followed by a character: that character with Alt.
    Alt(char),
    /// A cursor, editing or function key in VT form.
    Key { vk: u16, state: u32 },
    /// A win32-input-mode record, `CSI Vk;Sc;Uc;Kd;Cs;Rc _`.
    Win32(KeyEvent),
    /// A focus report, `CSI I` or `CSI O`.
    Focus(bool),
    /// A terminal's reply to a query (§5.6).
    Reply,
    /// A sequence with no meaning as input.
    Unknown,
}

/// A token with the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub token: Token,
    pub raw: Vec<u8>,
}

/// Splits terminal input into [`Piece`]s, keeping an unfinished sequence
/// or character for the next input.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    pending: Vec<u8>,
}

enum Parse {
    Done(Token, usize),
    Incomplete,
}

impl Decoder {
    /// Decode `bytes`. `end` says the terminal's write ended with them: an
    /// ESC at the very end is then the Escape key, while otherwise it is
    /// kept, as the rest of a sequence may follow.
    pub(crate) fn feed(&mut self, bytes: &[u8], end: bool) -> Vec<Piece> {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(bytes);
        let mut pieces = Vec::new();
        let mut at = 0;
        while at < buf.len() {
            match parse(&buf[at..], end) {
                Parse::Done(token, len) => {
                    pieces.push(Piece {
                        token,
                        raw: buf[at..at + len].to_vec(),
                    });
                    at += len;
                }
                Parse::Incomplete if buf.len() - at > MAX_PENDING => {
                    pieces.push(Piece {
                        token: Token::Unknown,
                        raw: buf[at..].to_vec(),
                    });
                    at = buf.len();
                }
                Parse::Incomplete => break,
            }
        }
        self.pending = buf.split_off(at);
        pieces
    }
}

fn parse(b: &[u8], end: bool) -> Parse {
    match b[0] {
        0x1b => escape(b, end),
        c @ (0x00..=0x1f | 0x7f) => Parse::Done(Token::Control(char::from(c)), 1),
        _ => text(b),
    }
}

/// A run of printable text up to the next control. An incomplete UTF-8
/// character at the end waits for more input; invalid bytes become U+FFFD.
fn text(b: &[u8]) -> Parse {
    let end = b
        .iter()
        .position(|&c| c < 0x20 || c == 0x7f)
        .unwrap_or(b.len());
    let run = &b[..end];
    let len = match std::str::from_utf8(run) {
        Ok(_) => end,
        Err(e) if e.error_len().is_none() && end == b.len() => e.valid_up_to(),
        Err(_) => end,
    };
    if len == 0 {
        return Parse::Incomplete;
    }
    let s = String::from_utf8_lossy(&run[..len]).into_owned();
    Parse::Done(Token::Text(s), len)
}

fn escape(b: &[u8], end: bool) -> Parse {
    // A lone ESC where the terminal's write ended is the Escape key: a
    // terminal writes a key's sequence whole. Anywhere else the rest of a
    // sequence may still be on its way.
    let Some(&next) = b.get(1) else {
        if end {
            return Parse::Done(Token::Control('\x1b'), 1);
        }
        return Parse::Incomplete;
    };
    match next {
        b'[' => csi(b),
        b'O' => match b.get(2) {
            Some(&f) => Parse::Done(ss3(f), 3),
            None => Parse::Incomplete,
        },
        b']' => string(b, true),
        b'P' => string(b, false),
        c @ (0x00..=0x1f | 0x7f) => Parse::Done(Token::Alt(char::from(c)), 2),
        _ => match text(&b[1..]) {
            Parse::Done(Token::Text(s), _) => {
                let ch = s.chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
                let n = if ch == char::REPLACEMENT_CHARACTER {
                    1
                } else {
                    ch.len_utf8()
                };
                Parse::Done(Token::Alt(ch), 1 + n)
            }
            _ => Parse::Incomplete,
        },
    }
}

/// An OSC (ended by BEL or ST) or DCS (ended by ST): a terminal reply.
fn string(b: &[u8], bel_ends: bool) -> Parse {
    let mut i = 2;
    while i < b.len() {
        match b[i] {
            0x07 if bel_ends => return Parse::Done(Token::Reply, i + 1),
            0x1b => match b.get(i + 1) {
                Some(b'\\') => return Parse::Done(Token::Reply, i + 2),
                Some(_) => return Parse::Done(Token::Unknown, i),
                None => return Parse::Incomplete,
            },
            _ => i += 1,
        }
    }
    Parse::Incomplete
}

fn csi(b: &[u8]) -> Parse {
    let mut i = 2;
    let prefix = match b.get(i) {
        Some(&c @ (b'<' | b'=' | b'>' | b'?')) => {
            i += 1;
            Some(c)
        }
        Some(_) => None,
        None => return Parse::Incomplete,
    };
    let params_at = i;
    while matches!(b.get(i), Some(0x30..=0x3f)) {
        i += 1;
    }
    let params = &b[params_at..i];
    let inter_at = i;
    while matches!(b.get(i), Some(0x20..=0x2f)) {
        i += 1;
    }
    let inter = &b[inter_at..i];
    let Some(&fin) = b.get(i) else {
        return Parse::Incomplete;
    };
    if !(0x40..=0x7e).contains(&fin) {
        // Malformed: give up on what came so far.
        return Parse::Done(Token::Unknown, i);
    }
    let token = match (prefix, inter, fin) {
        (Some(b'?' | b'>'), b"", b'c') | (_, b"$", b'y') => Token::Reply,
        (None, b"", b'_') => Token::Win32(win32(params)),
        (None, b"", b'I') if params.is_empty() => Token::Focus(true),
        (None, b"", b'O') if params.is_empty() => Token::Focus(false),
        (None, b"", _) => csi_key(params, fin).unwrap_or(Token::Unknown),
        _ => Token::Unknown,
    };
    Parse::Done(token, i + 1)
}

/// The numeric parameters; an empty one is `None`.
fn numbers(params: &[u8]) -> Vec<Option<u32>> {
    params
        .split(|&c| c == b';')
        .map(|p| {
            std::str::from_utf8(p)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        })
        .collect()
}

/// conhost `_GenerateWin32Key`: omitted values default to 0, the repeat
/// count to 1, and each is saturated to its field.
fn win32(params: &[u8]) -> KeyEvent {
    let n = numbers(params);
    let at = |i: usize, default: u32| n.get(i).copied().flatten().unwrap_or(default);
    let word = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    KeyEvent {
        down: at(3, 0) != 0,
        repeat: word(at(5, 1)),
        vk: word(at(0, 0)),
        scan: word(at(1, 0)),
        ch: word(at(2, 0)),
        state: at(4, 0),
    }
}

/// The xterm modifier parameter as `dwControlKeyState`.
fn modifiers(m: u32) -> u32 {
    let bits = m.saturating_sub(1);
    [
        (1, state::SHIFT),
        (2, state::LEFT_ALT),
        (4, state::LEFT_CTRL),
    ]
    .iter()
    .filter(|(bit, _)| bits & bit != 0)
    .fold(0, |s, (_, flag)| s | flag)
}

/// Cursor and editing keys carry ENHANCED_KEY.
fn enhanced(vk: u16) -> u32 {
    let is = matches!(
        vk,
        vk::PRIOR | vk::NEXT | vk::END | vk::HOME | vk::LEFT | vk::UP | vk::RIGHT | vk::DOWN
    ) || vk == vk::INSERT
        || vk == vk::DELETE;
    if is { state::ENHANCED_KEY } else { 0 }
}

fn cursor_vk(fin: u8) -> Option<u16> {
    Some(match fin {
        b'A' => vk::UP,
        b'B' => vk::DOWN,
        b'C' => vk::RIGHT,
        b'D' => vk::LEFT,
        b'H' => vk::HOME,
        b'F' => vk::END,
        b'P' => vk::F1,
        b'Q' => vk::F1 + 1,
        b'R' => vk::F1 + 2,
        b'S' => vk::F1 + 3,
        _ => return None,
    })
}

fn key(vk: u16, mods: u32) -> Token {
    Token::Key {
        vk,
        state: mods | enhanced(vk),
    }
}

/// `CSI A` .. `CSI 1;5D`, `CSI Z`, and the `CSI n ~` keys.
fn csi_key(params: &[u8], fin: u8) -> Option<Token> {
    let n = numbers(params);
    let first = n.first().copied().flatten();
    let mods = n.get(1).copied().flatten().map_or(0, modifiers);
    if fin == b'Z' && params.is_empty() {
        return Some(key(vk::TAB, state::SHIFT));
    }
    if fin == b'~' {
        let vk = match first? {
            1 => vk::HOME,
            2 => vk::INSERT,
            3 => vk::DELETE,
            4 => vk::END,
            5 => vk::PRIOR,
            6 => vk::NEXT,
            n @ 11..=15 => vk::F1 + (n - 11) as u16,
            n @ 17..=21 => vk::F1 + 5 + (n - 17) as u16,
            23 => vk::F1 + 10,
            24 => vk::F12,
            _ => return None,
        };
        return Some(key(vk, mods));
    }
    let vk = cursor_vk(fin)?;
    match (params.is_empty(), first) {
        (true, _) | (false, Some(1)) => Some(key(vk, mods)),
        _ => None,
    }
}

fn ss3(fin: u8) -> Token {
    cursor_vk(fin).map_or(Token::Unknown, |vk| key(vk, 0))
}

/// Raw input as conhost's `_writeString` stores it: one key-down per
/// UTF-16 unit, NUL taking the key that types it.
pub(crate) fn text_events(raw: &[u8], key_for: KeyFor) -> Vec<KeyEvent> {
    String::from_utf8_lossy(raw)
        .encode_utf16()
        .map(|u| match (u, key_for('\0')) {
            (0, Some((vk, state))) => KeyEvent {
                vk,
                state,
                ..KeyEvent::text(0)
            },
            _ => KeyEvent::text(u),
        })
        .collect()
}

/// conhost `_GenerateWrappedSequence`: the modifiers go down, the key goes
/// down and up, the modifiers come up.
fn wrapped(ch: u16, vk: u16, state: u32) -> Vec<KeyEvent> {
    let mods = [
        (state::SHIFT, vk::SHIFT),
        (state::LEFT_ALT, vk::MENU),
        (state::LEFT_CTRL, vk::CONTROL),
    ];
    let held: Vec<(u32, u16)> = mods.into_iter().filter(|(f, _)| state & f != 0).collect();
    let mut out = Vec::new();
    let mut now = 0;
    for &(flag, mvk) in &held {
        now |= flag;
        out.push(KeyEvent::key(true, mvk, 0, now));
    }
    out.push(KeyEvent::key(true, vk, ch, state));
    out.push(KeyEvent::key(false, vk, ch, state));
    for &(flag, mvk) in held.iter().rev() {
        now &= !flag;
        out.push(KeyEvent::key(false, mvk, 0, now));
    }
    out
}

/// conhost `_DoControlCharacter`.
fn control(c: char, alt: bool, key_for: KeyFor) -> Vec<KeyEvent> {
    let alt_state = if alt { state::LEFT_ALT } else { 0 };
    match c {
        // Ctrl+C is handled by the host, so it is never wrapped.
        '\x03' if !alt => vec![
            KeyEvent::key(true, vk::C, 3, state::LEFT_CTRL),
            KeyEvent::key(false, vk::C, 3, state::LEFT_CTRL),
        ],
        // DEL is Backspace, as most terminals send it.
        '\x7f' => wrapped(0x08, vk::BACK, alt_state),
        '\x1b' => wrapped(0x1b, vk::ESCAPE, alt_state),
        _ => {
            // Backspace (0x08) is Ctrl+Backspace, typing DEL.
            let (actual, keep_mods, ctrl) = match c {
                '\x08' => ('\x7f', false, true),
                '\r' => ('\r', false, false),
                '\t' => ('\t', true, false),
                _ => (c, true, true),
            };
            let Some((vk, mods)) = key_for(actual) else {
                return Vec::new();
            };
            let mut state = if keep_mods { mods } else { 0 };
            if ctrl {
                state |= state::LEFT_CTRL;
            }
            wrapped(actual as u16, vk, state | alt_state)
        }
    }
}

/// The key records a piece becomes when the client doesn't want VT.
/// Replies and unknown sequences become nothing.
pub(crate) fn key_events(token: &Token, key_for: KeyFor) -> Vec<KeyEvent> {
    match token {
        Token::Text(s) => s.encode_utf16().map(KeyEvent::text).collect(),
        Token::Control(c) => control(*c, false, key_for),
        Token::Alt(c) if c.is_control() => control(*c, true, key_for),
        Token::Alt(c) => {
            let Some((vk, mods)) = key_for(*c) else {
                return Vec::new();
            };
            let mut units = [0u16; 2];
            let ch = c.encode_utf16(&mut units)[0];
            wrapped(ch, vk, mods | state::LEFT_ALT)
        }
        Token::Key { vk, state } => {
            let ch = if *vk == vk::TAB { 0x09 } else { 0 };
            wrapped(ch, *vk, *state)
        }
        Token::Win32(k) => vec![*k],
        Token::Focus(_) | Token::Reply | Token::Unknown => Vec::new(),
    }
}

/// A key record as VT for a client in VIRTUAL_TERMINAL_INPUT mode, a
/// subset of conhost's `TerminalInput::HandleKey`: key-downs with text,
/// and the cursor, editing and function keys with xterm modifiers.
/// Key-ups and modifier keys produce nothing.
pub(crate) fn vt_for(k: &KeyEvent, utf8: &mut Utf8Units) -> Vec<u8> {
    if !k.down
        || matches!(
            k.vk,
            vk::SHIFT | vk::CONTROL | vk::MENU | vk::LWIN | vk::RWIN
        )
    {
        return Vec::new();
    }
    let alt = k.state & state::ALT != 0;
    let ctrl = k.state & state::CTRL != 0;
    let shift = k.state & state::SHIFT != 0;
    let mut out = Vec::new();
    // A synthesized key (VK_PACKET or no VK) types its character, NUL
    // included (conhost `HandleKey`).
    if k.ch != 0 || k.vk == 0 || k.vk == vk::PACKET {
        // AltGr arrives as Ctrl+Alt and types its character plainly. A
        // low surrogate's Alt went out with its high half.
        if alt && !ctrl && !utf8.pending() {
            out.push(0x1b);
        }
        utf8.push(k.ch, &mut out);
        return out;
    }
    utf8.flush(&mut out);
    let m = 1 + u32::from(shift) + 2 * u32::from(alt) + 4 * u32::from(ctrl);
    let with = |base: &str, fin: char| {
        if m == 1 {
            format!("\x1b[{base}{fin}")
        } else {
            format!("\x1b[{};{m}{fin}", if base.is_empty() { "1" } else { base })
        }
    };
    let seq = match k.vk {
        vk::UP => with("", 'A'),
        vk::DOWN => with("", 'B'),
        vk::RIGHT => with("", 'C'),
        vk::LEFT => with("", 'D'),
        vk::HOME => with("", 'H'),
        vk::END => with("", 'F'),
        vk::INSERT => with("2", '~'),
        vk::DELETE => with("3", '~'),
        vk::PRIOR => with("5", '~'),
        vk::NEXT => with("6", '~'),
        f @ vk::F1..=vk::F12 => {
            let i = f - vk::F1;
            if i < 4 {
                let fin = char::from(b'P' + i as u8);
                if m == 1 {
                    format!("\x1bO{fin}")
                } else {
                    format!("\x1b[1;{m}{fin}")
                }
            } else {
                let n = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(i - 4)];
                with(&n.to_string(), '~')
            }
        }
        _ => return out,
    };
    out.extend(seq.into_bytes());
    out
}

/// UTF-16 units to UTF-8 one at a time, as records carry them: a high
/// surrogate waits for its low half, and a lone surrogate becomes U+FFFD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Utf8Units {
    high: Option<u16>,
}

impl Utf8Units {
    pub(crate) fn push(&mut self, unit: u16, out: &mut Vec<u8>) {
        let high = self.high.take();
        if is_low_surrogate(unit) {
            let ch = high.map_or(char::REPLACEMENT_CHARACTER, |h| pair(h, unit));
            push_char(out, ch);
            return;
        }
        if high.is_some() {
            push_char(out, char::REPLACEMENT_CHARACTER);
        }
        if is_high_surrogate(unit) {
            self.high = Some(unit);
            return;
        }
        push_char(
            out,
            char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER),
        );
    }

    /// A waiting high surrogate, given up as U+FFFD.
    pub(crate) fn flush(&mut self, out: &mut Vec<u8>) {
        if self.high.take().is_some() {
            push_char(out, char::REPLACEMENT_CHARACTER);
        }
    }

    /// Whether a high surrogate waits.
    pub(crate) fn pending(&self) -> bool {
        self.high.is_some()
    }

    /// Forget a waiting high surrogate.
    pub(crate) fn clear(&mut self) {
        self.high = None;
    }
}

pub(crate) fn is_high_surrogate(u: u16) -> bool {
    (0xD800..0xDC00).contains(&u)
}

pub(crate) fn is_low_surrogate(u: u16) -> bool {
    (0xDC00..0xE000).contains(&u)
}

fn pair(high: u16, low: u16) -> char {
    let c = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
    char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER)
}

fn push_char(out: &mut Vec<u8>, ch: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

/// The INPUT_RECORD queue. Terminal input keeps it at [`MAX_RECORDS`];
/// WriteConsoleInput may take it past.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    records: VecDeque<Record>,
}

impl Queue {
    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Append; `true` when the oldest record had to be dropped.
    pub(crate) fn push(&mut self, r: Record) -> bool {
        let full = self.records.len() >= MAX_RECORDS;
        if full {
            self.records.pop_front();
        }
        self.records.push_back(r);
        full
    }

    /// conhost `InputBuffer::Write` (`append`) or `Prepend` for
    /// WriteConsoleInput: a single appended record may coalesce with the
    /// last. Unlike terminal input, a client's records are never dropped
    /// and are not held to [`MAX_RECORDS`], as in conhost (owner decision
    /// 2026-11-01, #1335).
    pub(crate) fn write(&mut self, records: &[Record], append: bool, coalesce: bool) {
        if let ([r], true) = (records, append && coalesce)
            && self.coalesce(r)
        {
            return;
        }
        if append {
            self.records.extend(records);
        } else {
            for r in records.iter().rev() {
                self.records.push_front(*r);
            }
        }
    }

    /// conhost `_CoalesceEvent`: a key-down repeating the last key-down
    /// adds to its repeat count; a buffer size or a mouse move replaces
    /// the last one.
    fn coalesce(&mut self, r: &Record) -> bool {
        let Some(last) = self.records.back_mut() else {
            return false;
        };
        match (last, r) {
            (Record::Key(last), Record::Key(k)) => {
                let same = last.down
                    && k.down
                    && (last.scan == k.scan || k.state & NLS_IME_CONVERSION != 0)
                    && last.ch == k.ch
                    && last.state == k.state
                    && !is_high_surrogate(k.ch)
                    && !is_low_surrogate(k.ch);
                if same {
                    last.repeat = last.repeat.wrapping_add(k.repeat);
                }
                same
            }
            (last @ Record::BufferSize { .. }, Record::BufferSize { .. }) => {
                *last = *r;
                true
            }
            (
                Record::Other {
                    event_type: MOUSE_EVENT,
                    event: last,
                },
                Record::Other {
                    event_type: MOUSE_EVENT,
                    event,
                },
            ) if mouse_moved(last) && mouse_moved(event) => {
                last[..4].copy_from_slice(&event[..4]);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Record> {
        self.records.iter()
    }

    /// Remove the first `n` records.
    pub(crate) fn drain_front(&mut self, n: usize) {
        self.records.drain(..n.min(self.records.len()));
    }

    /// The next record for a stream read, one repeat at a time: a key
    /// with a repeat count above 1 stays queued with one fewer.
    pub(crate) fn pop_stream(&mut self) -> Option<Record> {
        let front = self.records.front_mut()?;
        if let Record::Key(k) = front
            && k.repeat > 1
        {
            k.repeat -= 1;
            return Some(Record::Key(KeyEvent { repeat: 1, ..*k }));
        }
        self.records.pop_front()
    }
}

fn mouse_moved(event: &[u8; 16]) -> bool {
    u32::from_le_bytes([event[12], event[13], event[14], event[15]]) == MOUSE_MOVED
}

/// A US-English stand-in for `VkKeyScanW`, for tests and the fuzzer.
#[cfg(any(test, feature = "fuzz"))]
pub(crate) fn us_key_for(c: char) -> Option<(u16, u32)> {
    Some(match c {
        '\0' => (0x32, state::LEFT_CTRL | state::SHIFT),
        '\r' => (vk::RETURN, 0),
        '\t' => (vk::TAB, 0),
        '\x7f' => (vk::BACK, state::LEFT_CTRL),
        '\x01'..='\x1a' => (0x40 + c as u16, state::LEFT_CTRL),
        'a'..='z' => (c.to_ascii_uppercase() as u16, 0),
        'A'..='Z' => (c as u16, state::SHIFT),
        '0'..='9' | ' ' => (c as u16, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(bytes: &[u8]) -> Vec<Token> {
        Decoder::default()
            .feed(bytes, true)
            .into_iter()
            .map(|p| p.token)
            .collect()
    }

    #[test]
    fn text_controls_and_keys_are_split_with_their_bytes() {
        let pieces = Decoder::default().feed(b"hi\r\x1b[A\x1b[1;5C\x1bOP\x1b[3~\x1b[Z", true);
        let raw: Vec<&[u8]> = pieces.iter().map(|p| p.raw.as_slice()).collect();
        assert_eq!(
            raw,
            [
                &b"hi"[..],
                b"\r",
                b"\x1b[A",
                b"\x1b[1;5C",
                b"\x1bOP",
                b"\x1b[3~",
                b"\x1b[Z"
            ]
        );
        let t: Vec<Token> = pieces.into_iter().map(|p| p.token).collect();
        assert_eq!(t[0], Token::Text("hi".into()));
        assert_eq!(t[1], Token::Control('\r'));
        assert_eq!(
            t[2],
            Token::Key {
                vk: vk::UP,
                state: state::ENHANCED_KEY
            }
        );
        assert_eq!(
            t[3],
            Token::Key {
                vk: vk::RIGHT,
                state: state::LEFT_CTRL | state::ENHANCED_KEY
            }
        );
        assert_eq!(
            t[4],
            Token::Key {
                vk: vk::F1,
                state: 0
            }
        );
        assert_eq!(
            t[5],
            Token::Key {
                vk: vk::DELETE,
                state: state::ENHANCED_KEY
            }
        );
        assert_eq!(
            t[6],
            Token::Key {
                vk: vk::TAB,
                state: state::SHIFT
            }
        );
    }

    #[test]
    fn win32_records_take_conhost_defaults() {
        let t = tokens(b"\x1b[65;30;97;1;0;1_\x1b[17;;;1;8_\x1b[_");
        assert_eq!(
            t[0],
            Token::Win32(KeyEvent {
                down: true,
                repeat: 1,
                vk: 0x41,
                scan: 30,
                ch: 97,
                state: 0
            })
        );
        assert_eq!(
            t[1],
            Token::Win32(KeyEvent {
                down: true,
                repeat: 1,
                vk: 0x11,
                scan: 0,
                ch: 0,
                state: 8
            })
        );
        assert_eq!(t[2], Token::Win32(KeyEvent::key(false, 0, 0, 0)));
    }

    #[test]
    fn replies_focus_and_alt_are_recognised() {
        let t = tokens(b"\x1b[?62;22c\x1b[>0;10;1c\x1b[?2027;1$y\x1b]11;rgb:0/0/0\x1b\\\x1bP1$r0m\x1b\\\x1b[I\x1b[O\x1bx");
        assert_eq!(
            t,
            [
                Token::Reply,
                Token::Reply,
                Token::Reply,
                Token::Reply,
                Token::Reply,
                Token::Focus(true),
                Token::Focus(false),
                Token::Alt('x')
            ]
        );
    }

    #[test]
    fn split_sequences_and_characters_wait_for_the_rest() {
        let mut d = Decoder::default();
        assert!(d.feed(b"\x1b[1;", true).is_empty());
        assert_eq!(
            d.feed(b"2A", true)
                .into_iter()
                .map(|p| p.token)
                .collect::<Vec<_>>(),
            [Token::Key {
                vk: vk::UP,
                state: state::SHIFT | state::ENHANCED_KEY
            }]
        );
        let e = "é".as_bytes();
        assert!(d.feed(&e[..1], true).is_empty());
        assert_eq!(d.feed(&e[1..], true)[0].token, Token::Text("é".into()));
        // A lone ESC at the end of the input is the Escape key.
        assert_eq!(d.feed(b"\x1b", true)[0].token, Token::Control('\x1b'));
    }

    #[test]
    fn an_endless_sequence_is_given_up_at_the_pending_cap() {
        let mut d = Decoder::default();
        let mut osc = b"\x1b]0;".to_vec();
        osc.extend(vec![b'x'; MAX_PENDING]);
        let pieces = d.feed(&osc, true);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].token, Token::Unknown);
        assert!(d.pending.is_empty());
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters() {
        assert_eq!(tokens(b"a\xffb"), [Token::Text("a\u{fffd}b".into())]);
    }

    #[test]
    fn controls_become_conhost_keys() {
        // Ctrl+C: down and up, not wrapped in Ctrl presses.
        let c = key_events(&Token::Control('\x03'), us_key_for);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].vk, c[0].ch, c[0].state), (vk::C, 3, state::LEFT_CTRL));
        // Enter: no modifiers.
        let cr = key_events(&Token::Control('\r'), us_key_for);
        assert_eq!(cr.len(), 2);
        assert_eq!((cr[0].vk, cr[0].ch, cr[0].state), (vk::RETURN, 13, 0));
        // DEL is Backspace; 0x08 is Ctrl+Backspace typing DEL.
        let del = key_events(&Token::Control('\x7f'), us_key_for);
        assert_eq!((del[0].vk, del[0].ch), (vk::BACK, 8));
        let bs = key_events(&Token::Control('\x08'), us_key_for);
        assert_eq!(bs.len(), 4, "Ctrl down, key down and up, Ctrl up");
        assert_eq!(
            (bs[1].vk, bs[1].ch, bs[1].state),
            (vk::BACK, 0x7f, state::LEFT_CTRL)
        );
        // Ctrl+A is wrapped in Ctrl.
        let a = key_events(&Token::Control('\x01'), us_key_for);
        assert_eq!(a[0].vk, vk::CONTROL);
        assert_eq!((a[1].vk, a[1].ch), (0x41, 1));
    }

    #[test]
    fn text_is_key_downs_without_keys() {
        let k = key_events(&Token::Text("a😀".into()), us_key_for);
        assert_eq!(k.len(), 3, "one record per UTF-16 unit");
        assert!(k.iter().all(|k| k.down && k.vk == 0));
        assert_eq!(k[0].ch, u16::from(b'a'));
    }

    #[test]
    fn alt_letters_and_shift_tab_are_wrapped() {
        let a = key_events(&Token::Alt('x'), us_key_for);
        assert_eq!(a[0].vk, vk::MENU);
        assert_eq!(
            (a[1].vk, a[1].ch, a[1].state),
            (0x58, u16::from(b'x'), state::LEFT_ALT)
        );
        let tab = key_events(
            &Token::Key {
                vk: vk::TAB,
                state: state::SHIFT,
            },
            us_key_for,
        );
        assert_eq!((tab[1].vk, tab[1].ch), (vk::TAB, 9));
    }

    fn vt_for_one(k: &KeyEvent) -> Vec<u8> {
        vt_for(k, &mut Utf8Units::default())
    }

    #[test]
    fn vt_for_keeps_a_surrogate_pair_across_records() {
        let mut utf8 = Utf8Units::default();
        let down = |ch| KeyEvent::key(true, 0, ch, 0);
        let up = |ch| KeyEvent::key(false, 0, ch, 0);
        assert!(vt_for(&down(0xD83D), &mut utf8).is_empty());
        assert!(
            vt_for(&up(0xD83D), &mut utf8).is_empty(),
            "a key-up leaves it waiting"
        );
        assert_eq!(vt_for(&down(0xDE00), &mut utf8), "😀".as_bytes());
        // Alt goes out once, with the high half.
        let alt = |ch| KeyEvent::key(true, 0, ch, state::LEFT_ALT);
        assert_eq!(vt_for(&alt(0xD83D), &mut utf8), b"\x1b");
        assert_eq!(vt_for(&alt(0xDE00), &mut utf8), "😀".as_bytes());
        // Lone halves become U+FFFD, a waiting one before the next key.
        assert_eq!(vt_for(&down(0xDE00), &mut utf8), "\u{FFFD}".as_bytes());
        assert!(vt_for(&down(0xD83D), &mut utf8).is_empty());
        assert_eq!(
            vt_for(&down(u16::from(b'a')), &mut utf8),
            "\u{FFFD}a".as_bytes()
        );
        assert!(vt_for(&down(0xD83D), &mut utf8).is_empty());
        let left = KeyEvent::key(true, vk::LEFT, 0, 0);
        assert_eq!(vt_for(&left, &mut utf8), "\u{FFFD}\x1b[D".as_bytes());
    }

    #[test]
    fn a_split_anywhere_decodes_as_the_whole() {
        let inputs: [&[u8]; 4] = [
            b"\x1b[A",
            b"\x1b[27;1;27;1;0;1_",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\x",
            "a\x1bOP😀\x1b[1;5C".as_bytes(),
        ];
        for input in inputs {
            let whole = tokens(input);
            for cut in 1..input.len() {
                let mut d = Decoder::default();
                let mut got: Vec<Token> = d
                    .feed(&input[..cut], false)
                    .into_iter()
                    .map(|p| p.token)
                    .collect();
                got.extend(d.feed(&input[cut..], true).into_iter().map(|p| p.token));
                assert_eq!(got, whole, "{input:?} cut at {cut}");
            }
        }
    }

    #[test]
    fn a_trailing_esc_is_the_escape_key_only_where_the_write_ended() {
        let mut d = Decoder::default();
        assert!(d.feed(b"\x1b", false).is_empty(), "kept: the write goes on");
        assert_eq!(d.feed(b"[A", true)[0].token, tokens(b"\x1b[A")[0]);
        assert_eq!(d.feed(b"\x1b", true)[0].token, Token::Control('\x1b'));
    }

    #[test]
    fn vt_for_turns_records_back_into_vt() {
        let down = |vk, ch, state| KeyEvent::key(true, vk, ch, state);
        assert_eq!(vt_for_one(&down(0x41, 97, 0)), b"a");
        assert_eq!(vt_for_one(&down(0x41, 97, state::LEFT_ALT)), b"\x1ba");
        assert_eq!(vt_for_one(&down(vk::UP, 0, state::ENHANCED_KEY)), b"\x1b[A");
        assert_eq!(
            vt_for_one(&down(vk::LEFT, 0, state::LEFT_CTRL)),
            b"\x1b[1;5D"
        );
        assert_eq!(vt_for_one(&down(vk::DELETE, 0, state::SHIFT)), b"\x1b[3;2~");
        assert_eq!(vt_for_one(&down(vk::F1, 0, 0)), b"\x1bOP");
        assert_eq!(vt_for_one(&down(vk::F12, 0, 0)), b"\x1b[24~");
        assert!(vt_for_one(&KeyEvent::key(false, 0x41, 97, 0)).is_empty());
        assert!(vt_for_one(&down(vk::SHIFT, 0, state::SHIFT)).is_empty());
    }

    #[test]
    fn records_encode_as_input_record() {
        let r = Record::Key(KeyEvent {
            down: true,
            repeat: 2,
            vk: 0x41,
            scan: 30,
            ch: 97,
            state: 0x10,
        });
        assert_eq!(
            r.encode(),
            [
                1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0x41, 0, 30, 0, 97, 0, 0x10, 0, 0, 0
            ]
        );
        assert_eq!(
            &Record::Focus(true).encode()[..8],
            &[0x10, 0, 0, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            &Record::BufferSize {
                cols: 80,
                rows: 300
            }
            .encode()[..8],
            &[4, 0, 0, 0, 80, 0, 0x2C, 1]
        );
    }

    #[test]
    fn the_queue_drops_the_oldest_past_the_cap_and_streams_repeats() {
        let mut q = Queue::default();
        for i in 0..MAX_RECORDS {
            assert!(!q.push(Record::Key(KeyEvent::text(i as u16))));
        }
        assert!(q.push(Record::Key(KeyEvent::text(0xFFFF))));
        assert_eq!(q.len(), MAX_RECORDS);
        assert_eq!(q.iter().next(), Some(&Record::Key(KeyEvent::text(1))));
        let mut q = Queue::default();
        q.push(Record::Key(KeyEvent {
            repeat: 3,
            ..KeyEvent::text(u16::from(b'x'))
        }));
        let units: Vec<Record> = std::iter::from_fn(|| q.pop_stream()).collect();
        assert_eq!(units, vec![Record::Key(KeyEvent::text(u16::from(b'x'))); 3]);
    }

    #[test]
    fn raw_text_maps_nul_to_its_key() {
        let k = text_events(b"a\0", us_key_for);
        assert_eq!(k[0], KeyEvent::text(97));
        assert_eq!((k[1].vk, k[1].ch), (0x32, 0));
    }
}
