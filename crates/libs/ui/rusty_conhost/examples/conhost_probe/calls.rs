//! The console calls, through platform-windows' admitted surface. Each
//! returns the Win32 error on failure. Screen calls go to stdout's
//! handle, input calls to stdin's.
#![allow(unsafe_code)]

use std::io::Write as _;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use platform_windows::ffi::win32_surface as w;

pub(crate) use w::{CHAR_INFO, CONSOLE_SCREEN_BUFFER_INFOEX, COORD, SMALL_RECT};

/// A call's result: the value, or the Win32 error.
pub(crate) type R<T> = Result<T, i32>;

fn last_error() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
}

/// A BOOL result as `R<()>`.
fn ok(b: i32) -> R<()> {
    if b == 0 { Err(last_error()) } else { Ok(()) }
}

fn std_handle(output: bool) -> w::HANDLE {
    let which = if output {
        w::STD_OUTPUT_HANDLE
    } else {
        w::STD_INPUT_HANDLE
    };
    // SAFETY: GetStdHandle takes a constant and returns a handle value
    // (possibly null or invalid); nothing is dereferenced.
    unsafe { w::GetStdHandle(which) }
}

fn out() -> w::HANDLE {
    std_handle(true)
}

fn input() -> w::HANDLE {
    std_handle(false)
}

pub(crate) fn coord(x: i16, y: i16) -> COORD {
    COORD { X: x, Y: y }
}

// ---- modes and code pages ----

pub(crate) fn mode(output: bool) -> R<u32> {
    let mut mode = 0;
    // SAFETY: `mode` is a valid, writable u32 for the call.
    ok(unsafe { w::GetConsoleMode(std_handle(output), &mut mode) })?;
    Ok(mode)
}

pub(crate) fn set_mode(output: bool, mode: u32) -> R<()> {
    // SAFETY: plain values in, a BOOL out.
    ok(unsafe { w::SetConsoleMode(std_handle(output), mode) })
}

pub(crate) fn cp() -> u32 {
    // SAFETY: no arguments; returns the input code page.
    unsafe { w::GetConsoleCP() }
}

pub(crate) fn set_cp(cp: u32) -> R<()> {
    // SAFETY: a plain value in, a BOOL out.
    ok(unsafe { w::SetConsoleCP(cp) })
}

pub(crate) fn output_cp() -> u32 {
    // SAFETY: no arguments; returns the output code page.
    unsafe { w::GetConsoleOutputCP() }
}

pub(crate) fn set_output_cp(cp: u32) -> R<()> {
    // SAFETY: a plain value in, a BOOL out.
    ok(unsafe { w::SetConsoleOutputCP(cp) })
}

// ---- writing ----

/// `WriteFile` on stdout, which the console serves as a raw write;
/// std's console stdout would use WriteConsoleW instead.
pub(crate) fn write_raw(bytes: &[u8]) -> R<()> {
    use std::os::windows::io::FromRawHandle as _;
    // SAFETY: stdout's handle stays open for the process's life;
    // ManuallyDrop keeps the File from closing it.
    let file = unsafe { std::fs::File::from_raw_handle(out()) };
    let mut file = std::mem::ManuallyDrop::new(file);
    file.write_all(bytes)
        .map_err(|e| e.raw_os_error().unwrap_or(-1))
}

/// WriteConsoleW; the units written.
pub(crate) fn write_w(units: &[u16]) -> R<u32> {
    let mut n = 0;
    let len = u32::try_from(units.len()).map_err(|_| -1)?;
    // SAFETY: `units` is readable for `len` units; `n` is writable.
    ok(unsafe { w::WriteConsoleW(out(), units.as_ptr(), len, &mut n, std::ptr::null()) })?;
    Ok(n)
}

/// WriteConsoleA; the bytes written.
pub(crate) fn write_a(bytes: &[u8]) -> R<u32> {
    let mut n = 0;
    let len = u32::try_from(bytes.len()).map_err(|_| -1)?;
    // SAFETY: `bytes` is readable for `len` bytes; `n` is writable.
    ok(unsafe { w::WriteConsoleA(out(), bytes.as_ptr(), len, &mut n, std::ptr::null()) })?;
    Ok(n)
}

// ---- the screen buffer ----

pub(crate) fn sbi() -> R<w::CONSOLE_SCREEN_BUFFER_INFO> {
    // SAFETY: a plain-data struct, zero is a valid value.
    let mut info: w::CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is writable for the call.
    ok(unsafe { w::GetConsoleScreenBufferInfo(out(), &mut info) })?;
    Ok(info)
}

pub(crate) fn sbi_ex() -> R<CONSOLE_SCREEN_BUFFER_INFOEX> {
    // SAFETY: a plain-data struct, zero is a valid value.
    let mut info: CONSOLE_SCREEN_BUFFER_INFOEX = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<CONSOLE_SCREEN_BUFFER_INFOEX>() as u32;
    // SAFETY: `info` is writable and its cbSize is set.
    ok(unsafe { w::GetConsoleScreenBufferInfoEx(out(), &mut info) })?;
    Ok(info)
}

/// (size, visible).
pub(crate) fn cursor_info() -> R<(u32, bool)> {
    let mut info = w::CONSOLE_CURSOR_INFO {
        dwSize: 0,
        bVisible: 0,
    };
    // SAFETY: `info` is writable for the call.
    ok(unsafe { w::GetConsoleCursorInfo(out(), &mut info) })?;
    Ok((info.dwSize, info.bVisible != 0))
}

pub(crate) fn set_cursor_info(size: u32, visible: bool) -> R<()> {
    let info = w::CONSOLE_CURSOR_INFO {
        dwSize: size,
        bVisible: i32::from(visible),
    };
    // SAFETY: `info` is readable for the call.
    ok(unsafe { w::SetConsoleCursorInfo(out(), &info) })
}

pub(crate) fn set_cursor(at: COORD) -> R<()> {
    // SAFETY: plain values in, a BOOL out.
    ok(unsafe { w::SetConsoleCursorPosition(out(), at) })
}

pub(crate) fn set_attr(attr: u16) -> R<()> {
    // SAFETY: plain values in, a BOOL out.
    ok(unsafe { w::SetConsoleTextAttribute(out(), attr) })
}

pub(crate) fn set_buffer_size(size: COORD) -> R<()> {
    // SAFETY: plain values in, a BOOL out.
    ok(unsafe { w::SetConsoleScreenBufferSize(out(), size) })
}

pub(crate) fn set_window(absolute: bool, rect: SMALL_RECT) -> R<()> {
    // SAFETY: `rect` is readable for the call.
    ok(unsafe { w::SetConsoleWindowInfo(out(), i32::from(absolute), &rect) })
}

/// GetLargestConsoleWindowSize; a zero size is its failure.
pub(crate) fn largest() -> R<COORD> {
    // SAFETY: a handle in, a COORD out.
    let size = unsafe { w::GetLargestConsoleWindowSize(out()) };
    if size.X == 0 && size.Y == 0 {
        return Err(last_error());
    }
    Ok(size)
}

/// (index, cell size).
pub(crate) fn font(maximum: bool) -> R<(u32, COORD)> {
    let mut info = w::CONSOLE_FONT_INFO {
        nFont: 0,
        dwFontSize: coord(0, 0),
    };
    // SAFETY: `info` is writable for the call.
    ok(unsafe { w::GetCurrentConsoleFont(out(), i32::from(maximum), &mut info) })?;
    Ok((info.nFont, info.dwFontSize))
}

pub(crate) fn window_is_null() -> bool {
    // SAFETY: no arguments; the window handle is only compared.
    unsafe { w::GetConsoleWindow() }.is_null()
}

// ---- cells ----

/// FillConsoleOutputCharacterW; the cells written.
pub(crate) fn fill_chars(unit: u16, len: u32, at: COORD) -> R<u32> {
    let mut n = 0;
    // SAFETY: `n` is writable for the call.
    ok(unsafe { w::FillConsoleOutputCharacterW(out(), unit, len, at, &mut n) })?;
    Ok(n)
}

/// FillConsoleOutputAttribute; the cells written.
pub(crate) fn fill_attrs(attr: u16, len: u32, at: COORD) -> R<u32> {
    let mut n = 0;
    // SAFETY: `n` is writable for the call.
    ok(unsafe { w::FillConsoleOutputAttribute(out(), attr, len, at, &mut n) })?;
    Ok(n)
}

/// WriteConsoleOutputCharacterW; the cells written.
pub(crate) fn write_chars(units: &[u16], at: COORD) -> R<u32> {
    let mut n = 0;
    let len = u32::try_from(units.len()).map_err(|_| -1)?;
    // SAFETY: `units` is readable for `len` units; `n` is writable.
    ok(unsafe { w::WriteConsoleOutputCharacterW(out(), units.as_ptr(), len, at, &mut n) })?;
    Ok(n)
}

/// WriteConsoleOutputAttribute; the cells written.
pub(crate) fn write_attrs(attrs: &[u16], at: COORD) -> R<u32> {
    let mut n = 0;
    let len = u32::try_from(attrs.len()).map_err(|_| -1)?;
    // SAFETY: `attrs` is readable for `len` values; `n` is writable.
    ok(unsafe { w::WriteConsoleOutputAttribute(out(), attrs.as_ptr(), len, at, &mut n) })?;
    Ok(n)
}

/// ReadConsoleOutputCharacterW of `len` cells.
pub(crate) fn read_chars(len: usize, at: COORD) -> R<Vec<u16>> {
    let mut buf = vec![0u16; len];
    let mut n = 0;
    let cap = u32::try_from(len).map_err(|_| -1)?;
    // SAFETY: `buf` is writable for `cap` units; `n` is writable.
    ok(unsafe { w::ReadConsoleOutputCharacterW(out(), buf.as_mut_ptr(), cap, at, &mut n) })?;
    buf.truncate(n as usize);
    Ok(buf)
}

/// ReadConsoleOutputAttribute of `len` cells.
pub(crate) fn read_attrs(len: usize, at: COORD) -> R<Vec<u16>> {
    let mut buf = vec![0u16; len];
    let mut n = 0;
    let cap = u32::try_from(len).map_err(|_| -1)?;
    // SAFETY: `buf` is writable for `cap` values; `n` is writable.
    ok(unsafe { w::ReadConsoleOutputAttribute(out(), buf.as_mut_ptr(), cap, at, &mut n) })?;
    buf.truncate(n as usize);
    Ok(buf)
}

pub(crate) fn cell(unit: u16, attr: u16) -> CHAR_INFO {
    CHAR_INFO {
        Char: w::CHAR_INFO_0 { UnicodeChar: unit },
        Attributes: attr,
    }
}

/// The unit of a cell written with [`cell`] or read back.
pub(crate) fn unit_of(c: &CHAR_INFO) -> u16 {
    // SAFETY: both union fields are plain integers; the W calls fill
    // UnicodeChar.
    unsafe { c.Char.UnicodeChar }
}

/// WriteConsoleOutputW of a `size` block; the region written.
pub(crate) fn write_block(cells: &[CHAR_INFO], size: COORD, region: SMALL_RECT) -> R<SMALL_RECT> {
    assert_eq!(cells.len(), (size.X * size.Y) as usize, "block size");
    let mut region = region;
    // SAFETY: `cells` holds size.X * size.Y cells; `region` is writable.
    ok(unsafe { w::WriteConsoleOutputW(out(), cells.as_ptr(), size, coord(0, 0), &mut region) })?;
    Ok(region)
}

/// ReadConsoleOutputW into a `size` block; the cells and region read.
pub(crate) fn read_block(size: COORD, region: SMALL_RECT) -> R<(Vec<CHAR_INFO>, SMALL_RECT)> {
    let mut cells = vec![cell(0, 0); (size.X * size.Y) as usize];
    let mut region = region;
    // SAFETY: `cells` holds size.X * size.Y cells; `region` is writable.
    ok(unsafe {
        w::ReadConsoleOutputW(out(), cells.as_mut_ptr(), size, coord(0, 0), &mut region)
    })?;
    Ok((cells, region))
}

pub(crate) fn scroll(
    rect: SMALL_RECT,
    clip: Option<SMALL_RECT>,
    to: COORD,
    fill: CHAR_INFO,
) -> R<()> {
    let clip_ptr = clip.as_ref().map_or(std::ptr::null(), std::ptr::from_ref);
    // SAFETY: `rect`, `fill` and the optional clip are readable.
    ok(unsafe { w::ScrollConsoleScreenBufferW(out(), &rect, clip_ptr, to, &fill) })
}

// ---- titles and processes ----

pub(crate) fn set_title(units: &[u16]) -> R<()> {
    let mut z = units.to_vec();
    z.push(0);
    // SAFETY: `z` is NUL-terminated.
    ok(unsafe { w::SetConsoleTitleW(z.as_ptr()) })
}

/// GetConsoleTitleW into `cap` units: the count it returns and the
/// units up to the NUL.
pub(crate) fn title(cap: usize) -> R<(u32, Vec<u16>)> {
    let mut buf = vec![0u16; cap.max(1)];
    let cap = u32::try_from(cap).map_err(|_| -1)?;
    // SAFETY: `buf` is writable for at least `cap` units.
    let n = unsafe { w::GetConsoleTitleW(buf.as_mut_ptr(), cap) };
    if n == 0 {
        let e = last_error();
        if e != 0 {
            return Err(e);
        }
    }
    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    buf.truncate(end);
    Ok((n, buf))
}

/// GetConsoleProcessList with room for `cap` pids: the count returned.
pub(crate) fn process_count(cap: usize) -> R<u32> {
    let mut buf = vec![0u32; cap.max(1)];
    let cap = u32::try_from(cap).map_err(|_| -1)?;
    // SAFETY: `buf` is writable for at least `cap` pids.
    let n = unsafe { w::GetConsoleProcessList(buf.as_mut_ptr(), cap) };
    if n == 0 {
        return Err(last_error());
    }
    Ok(n)
}

// ---- input ----

pub(crate) fn key(unit: u16, down: bool) -> w::INPUT_RECORD {
    w::INPUT_RECORD {
        EventType: w::KEY_EVENT as u16,
        Event: w::INPUT_RECORD_0 {
            KeyEvent: w::KEY_EVENT_RECORD {
                bKeyDown: i32::from(down),
                wRepeatCount: 1,
                wVirtualKeyCode: 0,
                wVirtualScanCode: 0,
                uChar: w::KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                dwControlKeyState: 0,
            },
        },
    }
}

/// The unit a key record carries, if it is a key-down.
pub(crate) fn key_down_unit(r: &w::INPUT_RECORD) -> Option<u16> {
    if u32::from(r.EventType) != w::KEY_EVENT {
        return None;
    }
    // SAFETY: EventType says the union holds a key event, whose uChar
    // the W calls fill as UnicodeChar.
    let k = unsafe { r.Event.KeyEvent };
    if k.bKeyDown == 0 {
        return None;
    }
    // SAFETY: as above, uChar holds UnicodeChar.
    Some(unsafe { k.uChar.UnicodeChar })
}

pub(crate) fn input_count() -> R<u32> {
    let mut n = 0;
    // SAFETY: `n` is writable for the call.
    ok(unsafe { w::GetNumberOfConsoleInputEvents(input(), &mut n) })?;
    Ok(n)
}

/// WriteConsoleInputW; the records written.
pub(crate) fn write_input(records: &[w::INPUT_RECORD]) -> R<u32> {
    let mut n = 0;
    let len = u32::try_from(records.len()).map_err(|_| -1)?;
    // SAFETY: `records` is readable for `len` records; `n` is writable.
    ok(unsafe { w::WriteConsoleInputW(input(), records.as_ptr(), len, &mut n) })?;
    Ok(n)
}

/// PeekConsoleInputW or ReadConsoleInputW of up to `cap` records.
pub(crate) fn input_records(cap: usize, remove: bool) -> R<Vec<w::INPUT_RECORD>> {
    let mut buf = vec![key(0, false); cap];
    let mut n = 0;
    let cap = u32::try_from(cap).map_err(|_| -1)?;
    let f = if remove {
        w::ReadConsoleInputW
    } else {
        w::PeekConsoleInputW
    };
    // SAFETY: `buf` is writable for `cap` records; `n` is writable.
    ok(unsafe { f(input(), buf.as_mut_ptr(), cap, &mut n) })?;
    buf.truncate(n as usize);
    Ok(buf)
}

pub(crate) fn flush_input() -> R<()> {
    // SAFETY: a handle in, a BOOL out.
    ok(unsafe { w::FlushConsoleInputBuffer(input()) })
}

/// ReadConsoleW of up to `cap` units.
pub(crate) fn read_console(cap: usize) -> R<Vec<u16>> {
    let mut buf = vec![0u16; cap];
    let mut n = 0;
    let len = u32::try_from(cap).map_err(|_| -1)?;
    // SAFETY: `buf` is writable for `len` units; `n` is writable.
    ok(unsafe {
        w::ReadConsoleW(
            input(),
            buf.as_mut_ptr().cast(),
            len,
            &mut n,
            std::ptr::null(),
        )
    })?;
    buf.truncate(n as usize);
    Ok(buf)
}

// ---- control events ----

/// Ctrl+C events [`on_ctrl`] has seen.
static CTRL_C_SEEN: AtomicU32 = AtomicU32::new(0);
/// How long [`on_ctrl`] holds a CTRL_CLOSE.
static HOLD_MS: AtomicU64 = AtomicU64::new(0);

/// Count Ctrl+C; announce CTRL_CLOSE through the console, then hold the
/// handler, as a client that cleans up slowly does.
extern "system" fn on_ctrl(event: u32) -> i32 {
    if event == w::CTRL_C_EVENT {
        CTRL_C_SEEN.fetch_add(1, Ordering::Relaxed);
        return 1;
    }
    if event != w::CTRL_CLOSE_EVENT {
        return 0;
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "ctrl-close");
    let _ = out.flush();
    drop(out);
    std::thread::sleep(Duration::from_millis(HOLD_MS.load(Ordering::Relaxed)));
    1
}

fn install_handler() -> R<()> {
    // SAFETY: `on_ctrl` is a safe `extern "system"` fn of the handler's
    // signature; it touches only stdout and atomics.
    ok(unsafe { w::SetConsoleCtrlHandler(Some(on_ctrl), 1) })
}

pub(crate) fn hold_ctrl_close(ms: u64) -> R<()> {
    HOLD_MS.store(ms, Ordering::Relaxed);
    install_handler()
}

/// Accept Ctrl+C (the test runner starts us ignoring it) and count it.
pub(crate) fn count_ctrl_c() -> R<()> {
    // SAFETY: a null handler only clears this process's ignore flag.
    ok(unsafe { w::SetConsoleCtrlHandler(None, 0) })?;
    install_handler()
}

pub(crate) fn ctrl_c_seen() -> u32 {
    CTRL_C_SEEN.load(Ordering::Relaxed)
}

pub(crate) fn generate_ctrl_c(group: u32) -> R<()> {
    // SAFETY: plain values in, a BOOL out.
    ok(unsafe { w::GenerateConsoleCtrlEvent(w::CTRL_C_EVENT, group) })
}
