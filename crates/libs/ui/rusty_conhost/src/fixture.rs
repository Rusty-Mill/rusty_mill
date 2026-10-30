//! Fixtures (design §8.2): text files of console events and the effects
//! they must produce, with no timing. One step per line:
//!
//! ```text
//! # a comment
//! packet <function> <process> <object> <input_size> <output_size> <payload hex> [held=<n>]
//! input <bytes hex> [held=<n>]
//! input-more <bytes hex> [held=<n>]
//! resize <cols> <rows>
//! drained [held=<n>]
//! serve-reads [held=<n>]
//! hangup
//! exited
//! expect <effect>
//! ```
//!
//! `held` is how many bytes the output queue held when the engine
//! handled the event, which decides what the §7.4 budget admits; it is 0
//! when left out. `input` carries the terminal's bytes up to the end of
//! its write, `input-more` bytes of a write still going on. Packets get their
//! line number as their id. Each event is followed by the `expect` lines of its
//! effects, logs aside, then `expect vt <hex>` for any VT it produced.
//!
//! The engine records sessions in this format when `RUSTY_CONHOST_RECORD`
//! names a file. Recording is sanitised: CONNECT's title, application
//! name and directory are blanked, and SetConsoleTitle's text becomes
//! `title`. Output text and terminal input are kept, typed passwords
//! included, so review a recording before committing it. The test runner checks every file under `tests/fixtures/`, and
//! with `RUSTY_CONHOST_BLESS` set rewrites their `expect` lines.

use std::fmt::Write as _;

use crate::console::{Event, Packet};
use crate::msg::{self, api};

/// The largest fixture file.
pub(crate) const FIXTURE_CAP: usize = 64 * 1024;

/// CONNECT's counted strings start at `TitleLength`.
const SERVER_MSG_STRINGS_AT: usize = 34;

/// One recorded line for `ev`, `held` bytes being queued.
pub(crate) fn event_line(ev: &Event<'_>, held: usize) -> String {
    let mut line = match ev {
        Event::Packet(p) => packet_line(p),
        Event::Input(bytes) => format!("input {}", hex_or_dash(bytes)),
        Event::InputMore(bytes) => format!("input-more {}", hex_or_dash(bytes)),
        Event::Resize(size) => format!("resize {} {}", size.cols, size.rows),
        Event::OutputDrained => "drained".to_owned(),
        Event::ServeReads => "serve-reads".to_owned(),
        Event::ChildExited => "exited".to_owned(),
        Event::HangUp => "hangup".to_owned(),
        Event::GraceExpired => "grace-expired".to_owned(),
    };
    if held > 0 {
        let _ = write!(line, " held={held}");
    }
    line
}

fn packet_line(p: &Packet<'_>) -> String {
    format!(
        "packet {} {} {} {} {} {}",
        p.function,
        p.process,
        p.object,
        p.input_size,
        p.output_size,
        hex_or_dash(&sanitise(p))
    )
}

/// The payload as recorded, private strings blanked.
fn sanitise(p: &Packet<'_>) -> Vec<u8> {
    let mut payload = p.payload.to_vec();
    match p.function {
        msg::CONNECT => {
            if let Some(strings) = payload.get_mut(SERVER_MSG_STRINGS_AT..) {
                strings.fill(0);
            }
        }
        msg::USER_DEFINED => {
            if let Some(m) = msg::api_msg(p.payload)
                && m.number == api::SET_TITLE
            {
                let unicode = msg::bool_at(m.fixed, 0).unwrap_or(false);
                let text: Vec<u8> = if unicode {
                    "title".encode_utf16().flat_map(u16::to_le_bytes).collect()
                } else {
                    b"title".to_vec()
                };
                payload.truncate(p.payload.len() - m.variable.len());
                payload.extend(text);
            }
        }
        _ => {}
    }
    payload
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Hex, or `-` for no bytes, so every field is one word.
fn hex_or_dash(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "-".to_owned()
    } else {
        hex(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use platform::term::WinSize;

    use super::*;
    use crate::console::{Console, Effect, OutputQueue, Reply};

    /// The `expect` text of one effect; `None` for logs.
    fn effect_line(e: &Effect) -> Option<String> {
        let reply = |kind: &str, r: &Reply| {
            format!(
                "{kind} {:08x} {} {} {} {}",
                r.status as u32,
                r.information,
                hex_or_dash(&r.write_back),
                r.output_at,
                hex_or_dash(&r.output)
            )
        };
        match e {
            Effect::Reply(r) => Some(reply("reply", r)),
            Effect::Complete(r) => Some(reply("complete", r)),
            Effect::SetInputAvailable(on) => Some(format!("input-available {on}")),
            Effect::Ctrl { pid, event } => Some(format!("ctrl {pid} {event:?}")),
            // Ctrl-queue bookkeeping, not a driver effect.
            Effect::CtrlGone { .. } => None,
            Effect::CloseServer => Some("close".to_owned()),
            Effect::Eof => Some("eof".to_owned()),
            Effect::Log(_) => None,
        }
    }

    fn unhex(s: &str) -> Option<Vec<u8>> {
        if s == "-" {
            return Some(Vec::new());
        }
        if !s.len().is_multiple_of(2) {
            return None;
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect()
    }

    /// An event line split into its words and its `held`.
    fn split_held(line: &str) -> (Vec<&str>, usize) {
        let mut held = 0;
        let words = line
            .split_whitespace()
            .filter(|w| match w.strip_prefix("held=") {
                Some(n) => {
                    held = n.parse().expect("held=<n>");
                    false
                }
                None => true,
            })
            .collect();
        (words, held)
    }

    /// Run one event line; the lines its effects should match.
    fn run(console: &mut Console, n: usize, line: &str, payload: &mut Vec<u8>) -> Vec<String> {
        let (words, held) = split_held(line);
        let num = |i: usize| -> u64 { words[i].parse().expect("a number") };
        let ev = match words[0] {
            "packet" => {
                *payload = unhex(words[6]).expect("payload hex");
                Event::Packet(Packet {
                    id: (n as u64).to_le_bytes(),
                    function: num(1) as u32,
                    process: num(2) as usize,
                    object: num(3) as usize,
                    input_size: num(4) as u32,
                    output_size: num(5) as u32,
                    payload,
                })
            }
            "input" => {
                *payload = unhex(words[1]).expect("input hex");
                Event::Input(payload)
            }
            "input-more" => {
                *payload = unhex(words[1]).expect("input hex");
                Event::InputMore(payload)
            }
            "resize" => Event::Resize(WinSize {
                cols: num(1) as u16,
                rows: num(2) as u16,
            }),
            "drained" => Event::OutputDrained,
            "serve-reads" => Event::ServeReads,
            "hangup" => Event::HangUp,
            "grace-expired" => Event::GraceExpired,
            "exited" => Event::ChildExited,
            other => panic!("unknown step {other:?}"),
        };
        let mut out = OutputQueue::default();
        out.set_held(held);
        let mut lines: Vec<String> = console
            .handle(ev, &mut out)
            .iter()
            .filter_map(effect_line)
            .collect();
        let vt = out.take();
        if !vt.is_empty() {
            lines.push(format!("vt {}", hex(&vt)));
        }
        lines
    }

    /// Check (or with `bless`, rewrite) one fixture.
    fn check(path: &Path, bless: bool) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if text.len() > FIXTURE_CAP {
            return Err(format!("{} bytes, over the {FIXTURE_CAP} cap", text.len()));
        }
        let size = WinSize {
            rows: 30,
            cols: 120,
        };
        let mut console = Console::new(size, crate::input::us_key_for);
        let mut payload = Vec::new();
        let mut blessed = String::new();
        let mut expected: Vec<String> = Vec::new();
        let mut actual: Vec<String> = Vec::new();
        let mut at = 0;
        let compare = |at: usize, expected: &[String], actual: &[String]| {
            if expected == actual {
                return Ok(());
            }
            Err(format!(
                "after line {at}:\n expected {expected:#?}\n actual {actual:#?}"
            ))
        };
        for (n, line) in text.lines().enumerate() {
            if let Some(e) = line.strip_prefix("expect ") {
                expected.push(e.to_owned());
                continue;
            }
            if !bless {
                compare(at, &expected, &actual)?;
            }
            expected.clear();
            blessed.push_str(line);
            blessed.push('\n');
            if line.trim().is_empty() || line.starts_with('#') {
                actual.clear();
                continue;
            }
            at = n + 1;
            actual = run(&mut console, at, line, &mut payload);
            for e in &actual {
                let _ = writeln!(blessed, "expect {e}");
            }
        }
        if bless {
            return std::fs::write(path, blessed).map_err(|e| e.to_string());
        }
        compare(at, &expected, &actual)
    }

    fn fixtures() -> Vec<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("tests/fixtures")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .collect();
        files.sort();
        files
    }

    #[test]
    fn every_fixture_replays_to_its_expected_effects() {
        let bless = std::env::var_os("RUSTY_CONHOST_BLESS").is_some();
        let files = fixtures();
        assert!(!files.is_empty(), "no fixtures found");
        let failures: Vec<String> = files
            .iter()
            .filter_map(|f| {
                check(f, bless)
                    .err()
                    .map(|e| format!("{}: {e}", f.display()))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn recording_blanks_connect_strings_and_titles() {
        let mut connect = msg::server_msg("C:\\secret\\cmd.exe");
        connect[16] = 120;
        let p = Packet {
            id: [0; 8],
            process: 4,
            object: 0,
            function: msg::CONNECT,
            input_size: connect.len() as u32,
            output_size: 0,
            payload: &connect,
        };
        let clean = sanitise(&p);
        assert_eq!(clean[16], 120, "the sizes stay");
        assert!(clean[SERVER_MSG_STRINGS_AT..].iter().all(|b| *b == 0));
        assert!(msg::server_msg_info(&clean).is_ok_and(|i| i.title.is_empty()));

        let mut set_title = api::SET_TITLE.to_le_bytes().to_vec();
        set_title.extend(1u32.to_le_bytes());
        set_title.push(0);
        set_title.extend(b"C:\\Users\\someone");
        let p = Packet {
            function: msg::USER_DEFINED,
            input_size: set_title.len() as u32,
            payload: &set_title,
            ..p
        };
        let clean = sanitise(&p);
        assert!(clean.ends_with(b"\0title"));
        assert!(event_line(&Event::Packet(p), 7).ends_with(" held=7"));
    }

    #[test]
    fn unhex_round_trips_and_rejects_junk() {
        assert_eq!(unhex(&hex(b"\x00\xffab")), Some(b"\x00\xffab".to_vec()));
        assert_eq!(unhex("-"), Some(Vec::new()));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
