//! Named keys, encoded as a terminal's program reads them: xterm's encoding,
//! the one terminal programs expect from any terminal that sets
//! `TERM=xterm-256color`.

/// The modifiers held with a key.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Mods {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl Mods {
    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }

    /// xterm's modifier parameter: 1 plus a bit per modifier.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// A key without its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    /// A cursor key, or Home or End: `CSI x`, or `SS3 x` in application
    /// cursor mode.
    Cursor(u8),
    /// `CSI n ~`.
    Tilde(u8),
    /// F1 to F4: `SS3 x`.
    Function(u8),
}

/// The bytes `names` encode, in order, for a terminal whose program has
/// application cursor keys (DECCKM) on or off. Fails on the first name
/// that is not a key, or a modifier the key has no encoding with.
pub(crate) fn encode(names: &[String], app_cursor: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for name in names {
        let (mods, key) = parse(name)?;
        push(&mut out, mods, key, app_cursor).map_err(|why| format!("`{name}`: {why}"))?;
    }
    Ok(out)
}

/// A name such as `ctrl+alt+x`: modifiers, then a key.
fn parse(name: &str) -> Result<(Mods, Key), String> {
    let mut mods = Mods::default();
    let mut rest = name;
    // A lone `+` is the key, not a separator.
    while let Some((head, tail)) = rest.split_once('+').filter(|(_, t)| !t.is_empty()) {
        match head.to_ascii_lowercase().as_str() {
            "ctrl" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            _ => break,
        }
        rest = tail;
    }
    key(rest)
        .map(|key| (mods, key))
        .ok_or_else(|| format!("`{name}` is not a key"))
}

fn key(name: &str) -> Option<Key> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(Key::Char(c));
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "space" => Key::Char(' '),
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "up" => Key::Cursor(b'A'),
        "down" => Key::Cursor(b'B'),
        "right" => Key::Cursor(b'C'),
        "left" => Key::Cursor(b'D'),
        "home" => Key::Cursor(b'H'),
        "end" => Key::Cursor(b'F'),
        "insert" => Key::Tilde(2),
        "delete" => Key::Tilde(3),
        "page-up" => Key::Tilde(5),
        "page-down" => Key::Tilde(6),
        "f1" => Key::Function(b'P'),
        "f2" => Key::Function(b'Q'),
        "f3" => Key::Function(b'R'),
        "f4" => Key::Function(b'S'),
        "f5" => Key::Tilde(15),
        "f6" => Key::Tilde(17),
        "f7" => Key::Tilde(18),
        "f8" => Key::Tilde(19),
        "f9" => Key::Tilde(20),
        "f10" => Key::Tilde(21),
        "f11" => Key::Tilde(23),
        "f12" => Key::Tilde(24),
        _ => return None,
    })
}

fn push(out: &mut Vec<u8>, mods: Mods, key: Key, app_cursor: bool) -> Result<(), &'static str> {
    match key {
        Key::Cursor(x) | Key::Function(x) if mods.any() => {
            out.extend_from_slice(format!("\x1b[1;{}", mods.param()).as_bytes());
            out.push(x);
        }
        Key::Cursor(x) if !app_cursor => out.extend_from_slice(&[0x1b, b'[', x]),
        Key::Cursor(x) | Key::Function(x) => out.extend_from_slice(&[0x1b, b'O', x]),
        Key::Tilde(n) if mods.any() => {
            out.extend_from_slice(format!("\x1b[{n};{}~", mods.param()).as_bytes());
        }
        Key::Tilde(n) => out.extend_from_slice(format!("\x1b[{n}~").as_bytes()),
        Key::Char(c) => push_char(out, mods, c)?,
        Key::Enter | Key::Tab | Key::Backspace | Key::Escape => push_control(out, mods, key)?,
    }
    Ok(())
}

/// A character: shift picks its capital, ctrl its control code, alt
/// sends ESC first.
fn push_char(out: &mut Vec<u8>, mods: Mods, c: char) -> Result<(), &'static str> {
    let c = match mods.shift {
        false => c,
        true if c.is_uppercase() => c,
        true => capital(c).ok_or("shift needs a letter with one capital; send it instead")?,
    };
    if mods.alt {
        out.push(0x1b);
    }
    if !mods.ctrl {
        let mut buf = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        return Ok(());
    }
    out.push(match c {
        'a'..='z' | 'A'..='Z' => c.to_ascii_uppercase() as u8 & 0x1f,
        ' ' | '@' => 0,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' => 0x1f,
        '?' => 0x7f,
        _ => return Err("ctrl needs a letter or one of @ [ \\ ] ^ _ ? space"),
    });
    Ok(())
}

/// The one character that is `c`'s capital; `None` for a character without
/// case or one whose capital is several (`ß` is `SS`).
fn capital(c: char) -> Option<char> {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(one), None) if c.is_lowercase() => Some(one),
        _ => None,
    }
}

/// Enter, Tab, Backspace or Escape: alt sends ESC first; shift+tab is
/// back-tab and ctrl+backspace is BS; no other modifier changes them.
fn push_control(out: &mut Vec<u8>, mods: Mods, key: Key) -> Result<(), &'static str> {
    if mods.alt {
        out.push(0x1b);
    }
    let bytes: &[u8] = match (key, mods.ctrl, mods.shift) {
        (Key::Tab, false, true) => b"\x1b[Z",
        (Key::Backspace, true, false) => b"\x08",
        (_, true, _) | (_, _, true) => return Err("has no encoding with ctrl or shift"),
        (Key::Enter, ..) => b"\r",
        (Key::Tab, ..) => b"\t",
        (Key::Backspace, ..) => b"\x7f",
        _ => b"\x1b",
    };
    out.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str], app_cursor: bool) -> Result<Vec<u8>, String> {
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        encode(&names, app_cursor)
    }

    fn one(name: &str) -> Vec<u8> {
        keys(&[name], false).expect(name)
    }

    #[test]
    fn characters_and_named_keys() {
        assert_eq!(keys(&["l", "s", "enter"], false), Ok(b"ls\r".to_vec()));
        assert_eq!(one("é"), "é".as_bytes());
        assert_eq!(one("+"), b"+");
        assert_eq!(one("space"), b" ");
        assert_eq!(one("Tab"), b"\t");
        assert_eq!(one("backspace"), b"\x7f");
        assert_eq!(one("escape"), b"\x1b");
        assert_eq!(one("delete"), b"\x1b[3~");
        assert_eq!(one("page-down"), b"\x1b[6~");
        assert_eq!(one("f1"), b"\x1bOP");
        assert_eq!(one("f12"), b"\x1b[24~");
    }

    #[test]
    fn cursor_keys_follow_the_programs_mode() {
        assert_eq!(keys(&["up", "end"], false), Ok(b"\x1b[A\x1b[F".to_vec()));
        assert_eq!(keys(&["up", "end"], true), Ok(b"\x1bOA\x1bOF".to_vec()));
        assert_eq!(keys(&["ctrl+up"], true), Ok(b"\x1b[1;5A".to_vec()));
    }

    #[test]
    fn modifiers() {
        assert_eq!(one("ctrl+c"), b"\x03");
        assert_eq!(one("CTRL+C"), b"\x03");
        assert_eq!(one("ctrl+["), b"\x1b");
        assert_eq!(one("ctrl+space"), b"\x00");
        assert_eq!(one("alt+x"), b"\x1bx");
        assert_eq!(one("shift+a"), b"A");
        assert_eq!(one("shift+é"), "É".as_bytes());
        assert_eq!(one("ctrl+alt+a"), b"\x1b\x01");
        assert_eq!(one("alt+enter"), b"\x1b\r");
        assert_eq!(one("shift+tab"), b"\x1b[Z");
        assert_eq!(one("ctrl+backspace"), b"\x08");
        assert_eq!(one("shift+f5"), b"\x1b[15;2~");
        assert_eq!(one("ctrl+alt+shift+f1"), b"\x1b[1;8P");
        assert_eq!(one("alt+page-up"), b"\x1b[5;3~");
    }

    #[test]
    fn refuses_what_has_no_encoding() {
        for name in [
            "",
            "enterr",
            "hyper+x",
            "shift+",
            "ctrl+1",
            "ctrl++",
            "shift+1",
            "ctrl+enter",
            "shift+ß",
            "ctrl+shift+ß",
            "shift+\u{fb01}",
        ] {
            let error = keys(&["a", name], false).expect_err(name);
            assert!(error.starts_with(&format!("`{name}`")), "{error}");
        }
    }
}
