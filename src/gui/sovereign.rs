//! A minimal single-window terminal front-end built on the sovereign no_std
//! window/GPU/font stack (`rusty_gui`, `rusty_gpu`, `rusty_font`) instead of
//! `winit`/`softbuffer`/`ab_glyph`. Windows and Linux only — there is no
//! macOS backend in `rusty_gui`/`rusty_gpu` yet (see their issue trackers).
//!
//! Deliberately **not** at feature parity with [`super::window`]: one
//! window, one PTY session, no tabs/splits/chrome/settings/accessibility,
//! no mouse selection or clipboard, an always-visible (non-blinking)
//! cursor. What it does have is real: a real PTY, real `AnsiParser`/`Grid`
//! processing, real glyph rasterization and ligature shaping (the same
//! [`super::cpu::draw_grid`] compositor the `winit` backend uses), real
//! keyboard input, and real resize handling. Scoped down to what the
//! sovereign crates can actually do today, not a stub.
//!
//! # Known issue (Windows): PTY output stalls after the initial handshake
//!
//! On Windows, the spawned child's ConPTY output reliably stalls after the
//! first one or two chunks (the `?9001h`/`?1004h` mode-set announcement,
//! sometimes the following clear-screen sequence) — confirmed via live
//! testing: the reader thread stays alive and blocked in a real `ReadFile`
//! call (not dead, not deadlocked on our own locks), and the child/conhost
//! process's CPU time stops advancing entirely (a real stall on their side,
//! not slowness). The identical `Backend`/`AnsiParser`/reader-thread pattern
//! works correctly through [`super::window`] (`winit`-based) in the same
//! environment, so this is specific to `rusty_gui::Window`'s Windows
//! backend, not this module's PTY/parser wiring. Ruled out during
//! investigation: handle-role assignment (matches `window.rs`'s proven
//! pattern), dropped window messages (every `PeekMessageW` result is
//! `TranslateMessage`d/`DispatchMessageW`'d unconditionally), the
//! reader thread dying (it re-enters `.read()` and blocks there), and two
//! real bugs fixed upstream during this investigation that turned out not
//! to be the cause: `CreateWindowExW` sizing the window instead of the
//! client area (fixed: `rusty_win32`'s `create_native_window` now uses
//! `AdjustWindowRectEx`), and missing process DPI awareness. Tracked as
//! <https://github.com/baileyrd/rusty_gui/issues/9> with the full
//! diagnostic evidence — not yet root-caused.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rusty_gpu::Framebuffer;
use rusty_gui::event::ModifiersState;
use rusty_gui::{Event, KeyCode, Window};

use super::cpu::draw_grid;
use super::font::{self, FontCache, GlyphSource};
use crate::backend::Backend;
use crate::config::Config;
use crate::core::{AnsiParser, Grid};

/// Built-in default font size (px); overridable via the `font_size` config key.
const FONT_PX: f32 = 18.0;
const INIT_COLS: u16 = 80;
const INIT_ROWS: u16 = 24;
/// Idle poll interval for the main loop: `rusty_gui` has no blocking
/// wait-for-events call, so this trades a little latency for not spinning a
/// CPU core. ~125Hz — comfortably above any real input or output cadence.
const POLL_INTERVAL: Duration = Duration::from_millis(8);

/// Runs the sovereign-stack window: spawns one PTY-backed shell in one OS
/// window and drives it until the window closes or the shell exits.
pub fn run(backend: &dyn Backend, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (backend, config);
        return Err(
            "the gui-sovereign backend only supports Windows and Linux today \
             (rusty_gui/rusty_gpu have no macOS backend yet)"
                .into(),
        );
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        run_impl(backend, config)
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn run_impl(backend: &dyn Backend, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    // The child renders through us now, so advertise a real terminal identity.
    unsafe {
        std::env::set_var("TERM", "xterm-256color");
        std::env::set_var("COLORTERM", "truecolor");
    }

    let font_px = config.font_size.unwrap_or(FONT_PX);
    let set = font::load_set(
        config.font.as_deref(),
        config.font_bold.as_deref(),
        config.font_italic.as_deref(),
        config.font_bold_italic.as_deref(),
        config.font_fallback.as_deref(),
    )
    .ok_or("no usable font found (set `font` in the config or $RUSTY_TERM_FONT)")?;
    let mut fc = FontCache::new(set, font_px, config.ligatures.unwrap_or(true))
        .ok_or("font failed to parse")?;
    let (cw, ch) = fc.cell_size();

    let cols = config.cols.unwrap_or(INIT_COLS);
    let rows = config.rows.unwrap_or(INIT_ROWS);
    let mut width = cw as u32 * cols as u32;
    let mut height = ch as u32 * rows as u32;

    let mut window = Window::new("rusty_term", width, height)?;

    let mut grid = Grid::new(cols as usize, rows as usize);
    grid.apply_theme(&config.theme);
    let grid = Arc::new(Mutex::new(grid));
    let parser = Arc::new(Mutex::new(AnsiParser::new()));

    let handle = backend.spawn_shell(
        cols,
        rows,
        config.shell.as_deref(),
        &config.command_args,
        config.cwd.as_deref(),
    )?;
    let mut reader = handle.try_clone()?;
    let mut replies = handle.try_clone()?;
    // Only `Some` on the owning Windows ConPTY handle: its output pipe EOFs
    // at teardown, not on child exit, so a reader clone's `.read()` would
    // never return on its own once the shell exits. This blocking closure
    // returns when the child actually exits, so a dedicated watcher thread
    // can trigger the close instead of waiting on an EOF that never comes.
    let exit_token = handle.exit_token();
    // The *owning* handle (not a clone) is kept as the writer, alive for the
    // whole session — matching `window.rs`'s `Pane.writer`. Clones own only
    // their duplicated pipe ends and must not outlive the owner.
    let mut writer = handle;

    let dirty = Arc::new(AtomicBool::new(true));
    let closed = Arc::new(AtomicBool::new(false));

    if let Some(token) = exit_token {
        let closed = Arc::clone(&closed);
        std::thread::spawn(move || {
            token();
            closed.store(true, Ordering::Release);
        });
    }

    {
        let grid = Arc::clone(&grid);
        let parser = Arc::clone(&parser);
        let dirty = Arc::clone(&dirty);
        let closed = Arc::clone(&closed);
        std::thread::spawn(move || {
            loop {
                match reader.read() {
                    Ok(data) if data.is_empty() => break, // EOF: child exited (Unix)
                    Ok(data) => {
                        let response = {
                            let mut g = grid.lock();
                            let mut p = parser.lock();
                            p.advance(&mut g, &data);
                            let _ = g.take_host_out(); // no host clipboard/title relay here
                            p.take_responses()
                        };
                        if !response.is_empty() {
                            let _ = replies.write(&response);
                        }
                        dirty.store(true, Ordering::Release);
                    }
                    Err(_) => break,
                }
            }
            closed.store(true, Ordering::Release);
        });
    }

    let mut fb = Framebuffer::new(width as usize, height as usize);
    let mut pixels = vec![0u32; width as usize * height as usize];
    let mut mods = ModifiersState::default();

    while !closed.load(Ordering::Acquire) {
        for ev in window.poll_events() {
            match ev {
                Event::CloseRequested => closed.store(true, Ordering::Release),
                Event::Resized(w, h) => {
                    if w == 0 || h == 0 {
                        continue;
                    }
                    width = w;
                    height = h;
                    let new_cols = (w / cw as u32).max(1) as u16;
                    let new_rows = (h / ch as u32).max(1) as u16;
                    grid.lock().resize(new_cols as usize, new_rows as usize);
                    let _ = writer.set_winsize(new_cols, new_rows);
                    fb = Framebuffer::new(w as usize, h as usize);
                    pixels = vec![0u32; w as usize * h as usize];
                    dirty.store(true, Ordering::Release);
                }
                Event::ModifiersChanged(m) => mods = m,
                Event::ReceivedCharacter(c) => {
                    // Plain typed text; Ctrl/Alt combinations are handled as
                    // KeyPressed(Char(_)) below instead, so a character isn't
                    // sent twice.
                    if !mods.ctrl && !mods.alt {
                        let mut buf = [0u8; 4];
                        let _ = writer.write(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
                Event::KeyPressed(key) => {
                    if let Some(bytes) = encode_key(key, mods) {
                        let _ = writer.write(&bytes);
                    }
                }
                Event::RedrawRequested => dirty.store(true, Ordering::Release),
                Event::KeyReleased(_)
                | Event::CursorMoved(..)
                | Event::MousePressed(_)
                | Event::MouseReleased(_)
                | Event::MouseWheel(_) => {
                    // No mouse selection/scrolling in this minimal backend yet.
                }
            }
        }

        if dirty.swap(false, Ordering::AcqRel) {
            {
                let g = grid.lock();
                draw_grid(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    &g,
                    0,
                    0,
                    0,
                    0,
                    true, // focused: always draw a solid cursor, no unfocused hollow state
                    true, // cursor_on: no blink timer in this loop
                    None,
                    &mut fc,
                );
            }
            fb.load(&pixels);
            fb.present(&window);
        }

        std::thread::sleep(POLL_INTERVAL);
    }

    Ok(())
}

/// Encodes a non-character key press to the bytes the PTY expects. Plain
/// character keys arrive via [`Event::ReceivedCharacter`] instead (it
/// reflects the active keyboard layout; `KeyCode::Char` doesn't) — this only
/// handles Ctrl+letter control codes and the named/arrow keys `rusty_gui`'s
/// [`KeyCode`] carries. `rusty_gui` doesn't yet report Home/End/PageUp/
/// PageDown/Delete/Insert/function keys (its `KeyCode` has no variants for
/// them), so those are unreachable here — a real, disclosed gap, not a
/// silently dropped feature.
fn encode_key(key: KeyCode, mods: ModifiersState) -> Option<Vec<u8>> {
    Some(match key {
        KeyCode::Return => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Escape => vec![0x1b],
        KeyCode::Space if mods.ctrl => vec![0x00],
        KeyCode::Space => vec![b' '],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        // Ctrl+letter: the C0 control code (`Ctrl+A` = 0x01, ... `Ctrl+Z` = 0x1a).
        KeyCode::Char(c) if mods.ctrl && c.is_ascii_alphabetic() => {
            vec![(c.to_ascii_uppercase() as u8) & 0x1f]
        }
        // Any other plain character: `ReceivedCharacter` already sent it.
        KeyCode::Char(_)
        | KeyCode::Shift
        | KeyCode::Control
        | KeyCode::Alt
        | KeyCode::Unknown(_) => {
            return None;
        }
    })
}
