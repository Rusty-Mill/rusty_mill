//! The parity probes: each calls one API, or a short sequence around it,
//! and returns its result line. Every probe starts on a fresh console.

use crate::calls::{self, COORD, R, SMALL_RECT, coord};
use crate::text::{escape, unescape, unhex};

/// Run the probe `args[0]`; its result line.
pub(crate) fn run(args: &[String]) -> Result<String, String> {
    let name = args.first().ok_or("missing probe")?.as_str();
    let a = Args(&args[1..]);
    let body = match name {
        // Modes and code pages.
        "modes" => format!(
            "in={} out={}",
            hex(calls::mode(false)),
            hex(calls::mode(true))
        ),
        "set-mode" => {
            let output = a.str(0)? == "out";
            let set = calls::set_mode(output, a.hex(1)?);
            format!("{} after={}", done(set), hex(calls::mode(output)))
        }
        "cp" => format!("in={}", calls::cp()),
        "set-cp" => {
            let set = calls::set_cp(a.num(0)?);
            format!("{} after={}", done(set), calls::cp())
        }
        "out-cp" => format!("out={}", calls::output_cp()),
        "set-out-cp" => {
            let set = calls::set_output_cp(a.num(0)?);
            format!("{} after={}", done(set), calls::output_cp())
        }
        // Writing, and where it leaves the cursor and pen.
        "print" => print(a.hex(0)?, &a.text(1)?),
        "print-a" => {
            let set = calls::set_output_cp(a.num(0)?);
            let n = calls::write_a(&unhex(a.str(1)?)?);
            format!("cp={} n={} {}", done(set), val(n, |n| n.to_string()), pen())
        }
        "print-x" => print(a.hex(0)?, &vec![u16::from(b'x'); a.num(1)?]),
        // The screen buffer.
        "sbi" => sbi(),
        "sbi-ex" => sbi_ex(),
        "colors" => val(calls::sbi_ex(), |i| {
            let table: Vec<String> = i.ColorTable.iter().map(|c| format!("{c:x}")).collect();
            format!("table={}", table.join(","))
        }),
        "cursor-info" => cursor_info(),
        "set-cursor-info" => {
            let set = calls::set_cursor_info(a.num(0)?, a.num::<u8>(1)? != 0);
            format!("{} after={}", done(set), cursor_info())
        }
        "set-cursor" => {
            let set = calls::set_cursor(coord(a.num(0)?, a.num(1)?));
            format!("{} {}", done(set), pen())
        }
        "set-attr" => {
            let set = calls::set_attr(a.hex::<u16>(0)?);
            format!("{} {}", done(set), pen())
        }
        "set-buffer" => {
            let set = calls::set_buffer_size(coord(a.num(0)?, a.num(1)?));
            format!("{} after={}", done(set), sbi())
        }
        "set-window" => {
            let rect = rect(a.num(1)?, a.num(2)?, a.num(3)?, a.num(4)?);
            let set = calls::set_window(a.num::<u8>(0)? != 0, rect);
            format!("{} after={}", done(set), sbi())
        }
        "largest" => format!("size={}", val(calls::largest(), xy)),
        "font" => format!(
            "now={} max={}",
            val(calls::font(false), font),
            val(calls::font(true), font)
        ),
        "window" => format!("null={}", u8::from(calls::window_is_null())),
        // Cells.
        "fill" => fill(
            a.text(0)?[0],
            a.hex(1)?,
            a.num(2)?,
            coord(a.num(3)?, a.num(4)?),
        ),
        "cells" => cells(&a.text(0)?, &attrs(a.str(1)?)?, coord(a.num(2)?, a.num(3)?)),
        "block" => block(),
        "scroll" => scroll(a.num::<u8>(0)? != 0),
        // Titles and processes.
        "title" => title(),
        "set-title" => {
            let set = calls::set_title(&a.text(0)?);
            format!("{} {}", done(set), title())
        }
        "procs" => format!(
            "n={} short={}",
            val(calls::process_count(8), |n| n.to_string()),
            val(calls::process_count(0), |n| n.to_string())
        ),
        // Input.
        "input" => input(),
        "cooked" => cooked(&a.text(0)?),
        "ctrl-c" => ctrl_c(a.num(0)?),
        other => return Err(format!("unknown probe {other:?}")),
    };
    Ok(format!("{name} {body}"))
}

/// A probe's arguments after its name.
struct Args<'a>(&'a [String]);

impl Args<'_> {
    fn str(&self, i: usize) -> Result<&str, String> {
        self.0
            .get(i)
            .map(String::as_str)
            .ok_or_else(|| format!("missing argument {i}"))
    }

    fn num<T: std::str::FromStr>(&self, i: usize) -> Result<T, String> {
        let s = self.str(i)?;
        s.parse().map_err(|_| format!("bad number {s:?}"))
    }

    fn hex<T: TryFrom<u32>>(&self, i: usize) -> Result<T, String> {
        let s = self.str(i)?;
        let v = u32::from_str_radix(s, 16).map_err(|_| format!("bad hex {s:?}"))?;
        T::try_from(v).map_err(|_| format!("hex out of range {s:?}"))
    }

    fn text(&self, i: usize) -> Result<Vec<u16>, String> {
        unescape(self.str(i)?)
    }
}

// ---- formatting ----

fn val<T>(r: R<T>, f: impl Fn(T) -> String) -> String {
    r.map_or_else(|e| format!("error={e}"), f)
}

fn done(r: R<()>) -> String {
    val(r, |()| "ok".to_owned())
}

fn hex(r: R<u32>) -> String {
    val(r, |v| format!("{v:#x}"))
}

fn xy(c: COORD) -> String {
    format!("{},{}", c.X, c.Y)
}

fn rect(l: i16, t: i16, r: i16, b: i16) -> SMALL_RECT {
    SMALL_RECT {
        Left: l,
        Top: t,
        Right: r,
        Bottom: b,
    }
}

fn ltrb(r: SMALL_RECT) -> String {
    format!("{},{},{},{}", r.Left, r.Top, r.Right, r.Bottom)
}

fn font((index, size): (u32, COORD)) -> String {
    format!("{index}:{}", xy(size))
}

fn attr_list(attrs: &[u16]) -> String {
    attrs
        .iter()
        .map(|a| format!("{a:x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn attrs(list: &str) -> Result<Vec<u16>, String> {
    list.split(',')
        .map(|a| u16::from_str_radix(a, 16).map_err(|_| format!("bad attribute {a:?}")))
        .collect()
}

// ---- probes ----

/// The cursor and the pen, from GetConsoleScreenBufferInfo.
fn pen() -> String {
    val(calls::sbi(), |i| {
        format!(
            "cursor={} attr={:#x}",
            xy(i.dwCursorPosition),
            i.wAttributes
        )
    })
}

fn sbi() -> String {
    val(calls::sbi(), |i| {
        format!(
            "size={} cursor={} attr={:#x} window={} max={}",
            xy(i.dwSize),
            xy(i.dwCursorPosition),
            i.wAttributes,
            ltrb(i.srWindow),
            xy(i.dwMaximumWindowSize)
        )
    })
}

fn sbi_ex() -> String {
    val(calls::sbi_ex(), |i| {
        format!(
            "size={} cursor={} attr={:#x} window={} max={} popup={:#x} full={}",
            xy(i.dwSize),
            xy(i.dwCursorPosition),
            i.wAttributes,
            ltrb(i.srWindow),
            xy(i.dwMaximumWindowSize),
            i.wPopupAttributes,
            i.bFullscreenSupported
        )
    })
}

fn cursor_info() -> String {
    val(calls::cursor_info(), |(size, visible)| {
        format!("size={size} visible={}", u8::from(visible))
    })
}

/// Set the output mode, WriteConsoleW `units`, and report the count,
/// cursor and pen.
fn print(mode: u32, units: &[u16]) -> String {
    let set = calls::set_mode(true, mode);
    let n = calls::write_w(units);
    format!(
        "mode={} n={} {}",
        done(set),
        val(n, |n| n.to_string()),
        pen()
    )
}

/// Fill `len` cells with `unit` and `attr`, then read back the row.
fn fill(unit: u16, attr: u16, len: u32, at: COORD) -> String {
    let chars = calls::fill_chars(unit, len, at);
    let attrs = calls::fill_attrs(attr, len, at);
    format!(
        "chars={} attrs={} {}",
        val(chars, |n| n.to_string()),
        val(attrs, |n| n.to_string()),
        row(coord(0, at.Y), 16)
    )
}

/// The first `len` characters and attributes from `at`.
fn row(at: COORD, len: usize) -> String {
    format!(
        "text={} attrs={}",
        val(calls::read_chars(len, at), |u| escape(&u)),
        val(calls::read_attrs(len, at), |a| attr_list(&a))
    )
}

/// WriteConsoleOutputCharacterW and WriteConsoleOutputAttribute at `at`,
/// then read back.
fn cells(units: &[u16], attrs: &[u16], at: COORD) -> String {
    let wrote = calls::write_chars(units, at);
    let painted = calls::write_attrs(attrs, at);
    format!(
        "chars={} attrs={} {}",
        val(wrote, |n| n.to_string()),
        val(painted, |n| n.to_string()),
        row(at, units.len().max(attrs.len()) + 2)
    )
}

/// WriteConsoleOutputW a 3x2 block that hangs off the right edge, then
/// ReadConsoleOutputW a 4x3 block around it.
fn block() -> String {
    let size = calls::sbi().map(|i| i.dwSize).unwrap_or(coord(0, 0));
    let right = size.X - 1;
    let cells: Vec<_> = "abcdef"
        .encode_utf16()
        .zip([0x1e, 0x2f, 0x07, 0x4e, 0x70, 0x0c])
        .map(|(u, a)| calls::cell(u, a))
        .collect();
    let wrote = calls::write_block(&cells, coord(3, 2), rect(right - 1, 1, right + 1, 2));
    let back = calls::read_block(coord(4, 3), rect(right - 2, 0, right + 1, 2));
    format!(
        "wrote={} read={}",
        val(wrote, ltrb),
        val(back, |(cells, region)| {
            let units: Vec<u16> = cells.iter().map(calls::unit_of).collect();
            let attrs: Vec<u16> = cells.iter().map(|c| c.Attributes).collect();
            format!(
                "{} text={} attrs={}",
                ltrb(region),
                escape(&units),
                attr_list(&attrs)
            )
        })
    )
}

/// Paint "abcdef" on row 0, scroll columns 0..=5 of it to (2,3), with a
/// clip rectangle when `clipped`, and read rows 0 and 3 back.
fn scroll(clipped: bool) -> String {
    let painted = calls::write_chars(&"abcdef".encode_utf16().collect::<Vec<_>>(), coord(0, 0));
    let clip = clipped.then(|| rect(0, 0, 4, 10));
    let moved = calls::scroll(
        rect(0, 0, 5, 0),
        clip,
        coord(2, 3),
        calls::cell(u16::from(b'.'), 0x1e),
    );
    format!(
        "painted={} {} row0=[{}] row3=[{}]",
        val(painted, |n| n.to_string()),
        done(moved),
        row(coord(0, 0), 9),
        row(coord(0, 3), 9)
    )
}

fn title() -> String {
    let full = calls::title(256);
    let short = calls::title(4);
    format!(
        "full={} short={}",
        val(full, |(n, t)| format!("{n}:{}", escape(&t))),
        val(short, |(n, t)| format!("{n}:{}", escape(&t)))
    )
}

/// Queue three key records, then count, peek, read one, and flush.
fn input() -> String {
    let before = calls::input_count();
    let wrote = calls::write_input(&[
        calls::key(u16::from(b'a'), true),
        calls::key(u16::from(b'a'), false),
        calls::key(u16::from(b'b'), true),
    ]);
    let queued = calls::input_count();
    let peeked = calls::input_records(8, false);
    // A read with nothing queued would block: read only what a peek saw.
    let read = match &peeked {
        Ok(rs) if !rs.is_empty() => calls::input_records(1, true),
        _ => Ok(Vec::new()),
    };
    let left = calls::input_count();
    let flushed = calls::flush_input();
    let after = calls::input_count();
    let units = |rs: Vec<_>| -> String {
        let downs: Vec<u16> = rs.iter().filter_map(calls::key_down_unit).collect();
        format!("{}:{}", rs.len(), escape(&downs))
    };
    let count = |r| val(r, |n: u32| n.to_string());
    format!(
        "before={} wrote={} queued={} peek={} read={} left={} flush={} after={}",
        count(before),
        count(wrote),
        count(queued),
        val(peeked, units),
        val(read, units),
        count(left),
        done(flushed),
        count(after)
    )
}

/// Type `units` as key records, then one cooked ReadConsoleW.
fn cooked(units: &[u16]) -> String {
    let records: Vec<_> = units
        .iter()
        .flat_map(|&u| [calls::key(u, true), calls::key(u, false)])
        .collect();
    let wrote = calls::write_input(&records);
    // Without the typed line the read would block.
    let read = match wrote {
        Ok(n) if n as usize == records.len() => calls::read_console(64),
        _ => Ok(Vec::new()),
    };
    format!(
        "wrote={} read={} {}",
        val(wrote, |n| n.to_string()),
        val(read, |u| escape(&u)),
        pen()
    )
}

/// GenerateConsoleCtrlEvent(CTRL_C_EVENT, group) and whether our
/// handler saw it.
fn ctrl_c(group: u32) -> String {
    if let Err(e) = calls::count_ctrl_c() {
        return format!("handler error={e}");
    }
    let sent = calls::generate_ctrl_c(group);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while sent.is_ok() && calls::ctrl_c_seen() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    format!("{} seen={}", done(sent), calls::ctrl_c_seen())
}
