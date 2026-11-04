//! The store's manifest: the saved model and how each block was started, as
//! lines of space-separated fields. Pure text in, text out.
//!
//! ```text
//! rusty_mux state 1
//! s <session> <generated 0|1> <active window|-> =<name>
//! w <window> <zoomed|-> <focused|-> <layout|-> =<name>
//! b <block> <cols> <rows> <shell 0|1> =<cwd> =<name> =<program> [=<arg>...]
//! ```
//!
//! Each `w` belongs to the `s` above it and each `b` to the `w` above it.
//! A text field starts with `=`, so an empty one is still a field, and
//! `%`, space and control characters are written `%XX`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rusty_mux_core::{
    BlockId, Layout, Saved, SavedBlock, SavedSession, SavedWindow, SessionId, Size, WindowId,
};
use rusty_uuid::Uuid;

/// The first line; another version is refused, never guessed at.
pub(crate) const HEADER: &str = "rusty_mux state 1";

/// How a block's program was started, so a later run can start another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Started {
    /// Started with the server's configured program, not one the request named.
    pub(crate) shell: bool,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    /// Its last known working directory.
    pub(crate) cwd: String,
}

/// The saved model with each block's [`Started`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) saved: Saved,
    pub(crate) started: BTreeMap<BlockId, Started>,
}

/// Why a manifest could not be read: the line and what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bad {
    pub(crate) line: usize,
    pub(crate) what: &'static str,
}

impl std::fmt::Display for Bad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "manifest line {}: {}", self.line, self.what)
    }
}

pub(crate) fn encode(manifest: &Manifest) -> String {
    let mut out = format!("{HEADER}\n");
    for s in &manifest.saved.sessions {
        let flag = u8::from(s.generated);
        let _ = writeln!(out, "s {} {flag} {} {}", s.id, opt(s.active), text(&s.name));
        for w in &s.windows {
            let layout = w.layout.as_ref().map_or("-".into(), Layout::to_string);
            let (zoomed, focused) = (opt(w.zoomed), opt(w.focused));
            let _ = writeln!(
                out,
                "w {} {zoomed} {focused} {layout} {}",
                w.id,
                text(&w.name)
            );
            for b in &w.blocks {
                encode_block(&mut out, b, manifest.started.get(&b.id));
            }
        }
    }
    out
}

fn encode_block(out: &mut String, b: &SavedBlock, started: Option<&Started>) {
    let (cols, rows) = (b.size.cols(), b.size.rows());
    let fallback = Started {
        shell: true,
        program: String::new(),
        args: Vec::new(),
        cwd: String::new(),
    };
    let s = started.unwrap_or(&fallback);
    let _ = write!(
        out,
        "b {} {cols} {rows} {} {} {} {}",
        b.id,
        u8::from(s.shell),
        text(&s.cwd),
        text(&b.name),
        text(&s.program)
    );
    for arg in &s.args {
        let _ = write!(out, " {}", text(arg));
    }
    out.push('\n');
}

fn opt(id: Option<impl std::fmt::Display>) -> String {
    id.map_or("-".into(), |id| id.to_string())
}

fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    out.push('=');
    for c in s.chars() {
        if c == '%' || c == ' ' || c.is_control() {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn decode(input: &str) -> Result<Manifest, Bad> {
    let mut lines = input.lines().enumerate().map(|(i, l)| (i + 1, l));
    match lines.next() {
        Some((_, HEADER)) => {}
        _ => {
            return Err(Bad {
                line: 1,
                what: "not a version 1 manifest",
            });
        }
    }
    let mut manifest = Manifest::default();
    for (line, text) in lines {
        let bad = |what| Bad { line, what };
        let mut fields = text.split(' ');
        match fields.next() {
            Some("s") => manifest
                .saved
                .sessions
                .push(session(&mut fields).ok_or(bad("bad session"))?),
            Some("w") => {
                let session = manifest
                    .saved
                    .sessions
                    .last_mut()
                    .ok_or(bad("window before any session"))?;
                session
                    .windows
                    .push(window(&mut fields).ok_or(bad("bad window"))?);
            }
            Some("b") => {
                let window = manifest
                    .saved
                    .sessions
                    .last_mut()
                    .and_then(|s| s.windows.last_mut())
                    .ok_or(bad("block before any window"))?;
                let (block, started) = block(&mut fields).ok_or(bad("bad block"))?;
                manifest.started.insert(block.id, started);
                window.blocks.push(block);
            }
            _ => return Err(bad("unknown line")),
        }
    }
    Ok(manifest)
}

type Fields<'a> = std::str::Split<'a, char>;

fn session(f: &mut Fields<'_>) -> Option<SavedSession> {
    let id = SessionId::from_uuid(uuid(f.next()?)?);
    let generated = flag(f.next()?)?;
    let active = maybe(f.next()?, |s| uuid(s).map(WindowId::from_uuid))?;
    let name = unescape(f.next()?)?;
    f.next().is_none().then_some(SavedSession {
        id,
        name,
        generated,
        active,
        windows: Vec::new(),
    })
}

fn window(f: &mut Fields<'_>) -> Option<SavedWindow> {
    let id = WindowId::from_uuid(uuid(f.next()?)?);
    let zoomed = maybe(f.next()?, |s| uuid(s).map(BlockId::from_uuid))?;
    let focused = maybe(f.next()?, |s| uuid(s).map(BlockId::from_uuid))?;
    let layout = maybe(f.next()?, |s| s.parse().ok())?;
    let name = unescape(f.next()?)?;
    f.next().is_none().then_some(SavedWindow {
        id,
        name,
        layout,
        zoomed,
        focused,
        blocks: Vec::new(),
    })
}

fn block(f: &mut Fields<'_>) -> Option<(SavedBlock, Started)> {
    let id = BlockId::from_uuid(uuid(f.next()?)?);
    let cols = f.next()?.parse().ok()?;
    let rows = f.next()?.parse().ok()?;
    let size = Size::new(cols, rows).ok()?;
    let shell = flag(f.next()?)?;
    let cwd = unescape(f.next()?)?;
    let name = unescape(f.next()?)?;
    let program = unescape(f.next()?)?;
    let args = f.map(unescape).collect::<Option<_>>()?;
    let started = Started {
        shell,
        program,
        args,
        cwd,
    };
    Some((SavedBlock { id, name, size }, started))
}

fn uuid(s: &str) -> Option<Uuid> {
    s.parse().ok()
}

fn flag(s: &str) -> Option<bool> {
    match s {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// `-` is `None`; anything else must parse.
fn maybe<T>(s: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<Option<T>> {
    if s == "-" {
        return Some(None);
    }
    parse(s).map(Some)
}

fn unescape(s: &str) -> Option<String> {
    let s = s.strip_prefix('=')?;
    let mut bytes = Vec::with_capacity(s.len());
    let mut rest = s.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        if b != b'%' {
            bytes.push(b);
            rest = tail;
            continue;
        }
        let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
        bytes.push(u8::from_str_radix(hex, 16).ok()?);
        rest = &tail[2..];
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::{Mux, Side};

    use super::*;

    fn manifest() -> Manifest {
        let mut mux = Mux::new();
        let s = mux.create_session("my work").unwrap();
        let w = mux.create_window(s, "").unwrap();
        let a = mux.create_block(w, Size::new(80, 24).unwrap()).unwrap();
        let b = BlockId::from_uuid(Uuid::new_v4());
        mux.split_block(b, a, Side::Down, Size::new(80, 11).unwrap())
            .unwrap();
        mux.rename(rusty_mux_core::EntityId::Block(b), "100% =\n\u{e9}")
            .unwrap();
        mux.create_session("empty").unwrap();
        let started = |shell, args: &[&str]| Started {
            shell,
            program: "/bin/sh".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: "/home/a b".into(),
        };
        Manifest {
            saved: mux.save(),
            started: BTreeMap::from([
                (a, started(true, &[])),
                (b, started(false, &["-c", "", "x y"])),
            ]),
        }
    }

    #[test]
    fn round_trips() {
        let manifest = manifest();
        let text = encode(&manifest);
        assert!(text.contains("=100%25%20=%0A\u{e9}"), "{text}");
        assert_eq!(decode(&text), Ok(manifest));
    }

    #[test]
    fn refuses_what_it_cannot_read_whole() {
        let text = encode(&manifest());
        let bad = |t: &str| decode(t).unwrap_err().what;
        assert_eq!(bad("rusty_mux state 2\n"), "not a version 1 manifest");
        assert_eq!(
            bad(&format!("{HEADER}\nw x\n")),
            "window before any session"
        );
        assert_eq!(bad(&format!("{HEADER}\nq\n")), "unknown line");
        assert_eq!(bad(&text.replace(" 80 24 ", " 0 24 ")), "bad block");
        assert_eq!(bad(&text.replacen(" =my", " my", 1)), "bad session");
        assert_eq!(bad(&text.replace("=/home/a%20b", "=%G0")), "bad block");
        assert_eq!(bad(&text.replacen("rows", "nope", 1)), "bad window");
    }
}
