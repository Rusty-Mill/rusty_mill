# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A single-file Rust library crate (`src/lib.rs`): a hand-rolled readline
alternative extracted from the [rush shell](https://github.com/baileyrd/rush).
Only two dependencies — `unicode-width` and (Unix-only) `libc`. Do not add
editing/terminal crates; being dependency-free is the point.

## Commands

```sh
cargo test                          # unit tests + pty tests + doctest
cargo test --test pty               # just the end-to-end pty suite
cargo test up_arrow_recalls_history # single test by name
cargo run --example demo            # interactive REPL to try changes by hand
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI (`.github/workflows/ci.yml`) also runs `cargo doc --no-deps` with
`RUSTDOCFLAGS=-D warnings`, an MSRV check (`rust-version` in Cargo.toml — keep
the CI job, Cargo.toml, and CHANGELOG in sync when bumping), and a Windows
build. Locally, the Windows path can be checked with
`cargo check --all-targets --target x86_64-pc-windows-gnu`.

## Architecture

Everything lives in `src/lib.rs` (~2k lines). Public surface: `Editor`,
`Hooks` (host integration trait — completion, hints, highlighting,
abbreviations, vi-mode flag, external editor, EINTR callback), `NoHooks`,
`ReadResult`, `Candidate`.

Layers inside `read_line`, bottom to top:

- **Raw mode / bracketed paste** behind RAII guards, so every exit path
  (including panics) restores the terminal.
- **Key decoding**: UTF-8 assembly + CSI/SS3 escape parsing, with a short
  poll to distinguish a lone ESC from a sequence.
- **Render engine**: a full (not diffed) repaint of the edit region;
  display-width math is ANSI-aware and control chars render `^X`-style, so
  cursor math must stay exact — width bugs show up as misplaced cursors.
  Not synchronously tied to every keystroke: the main loop coalesces —
  skips a repaint when more input is already queued, since it would be
  instantly overwritten by the next key's repaint anyway (readline's
  trick, capped so a very large flood still shows periodic progress).
  `LineState::render_owed` tracks a pending coalesced paint;
  `finish_line` — the single choke point every exit path funnels
  through — flushes it before computing cursor-repositioning math, which
  would otherwise corrupt the display against stale row/column
  bookkeeping. Any new path that can end a read (or otherwise assumes
  the terminal cursor matches `painted_rows`/`painted_cursor_row`) must
  go through `finish_line`, not print directly.
- **Keymaps**: emacs by default, vi when `Hooks::vi_mode()` is true
  (checked live per `read_line`, no editor rebuild).
- **History**: multi-line entries are stored joined with `; ` (bash
  `cmdhist`); `load_history` tolerates a rustyline `#V2` header.

Platform split: the raw editor runs on both Unix and Windows — the only
platform-specific code is `term_sys.rs`, which exposes one interface
(`isatty_*`/`tcgetattr_stdin`/`tcsetattr_stdin_drain`/`apply_raw_flags`/
`is_raw`/`poll_stdin`/`read_stdin_chunk`/`term_*_stdout`/`clear_echo_flag`)
backed by `libc`/`rusty_libc` (Unix) or `rusty_win32` (Windows,
`GetConsoleMode`/`SetConsoleMode`/`ReadFile`/`WaitForSingleObject`/
`GetConsoleScreenBufferInfo` — deliberately not ConPTY, which hosts a
*child* process's console rather than this process's own). `lib.rs` itself
never touches `libc`/`rusty_libc`/`rusty_win32`/`std::os::{unix,windows}`
directly except for the history-file permission-bit calls (`0600` on Unix,
no Windows equivalent — those few sites keep their own narrow
`#[cfg(unix)]` gates). Non-tty stdin falls back to a plain read on both
platforms. The Windows path has **no pseudo-terminal-driven behavioral
test coverage** (`tests/pty.rs` is `#![cfg(unix)]`-only) — only
compilation, `term_sys.rs`'s own pure bit-math unit tests, and whatever
`rusty_win32` already verifies at the primitive layer on real
`windows-latest` CI. Real interactive verification on Windows is still
outstanding; treat Windows raw-mode changes with more caution than Unix
ones for exactly that reason.

The README's "Deliberate narrowings" section lists features that are
intentionally NOT supported (multi-line buffer editing, programmable
keybindings, keyboard macros, completion paging, …). Don't add them without
being asked. `docs/survey.md` is the fuller per-editor audit behind the
README's feature matrix — when adding or declining a capability, update the
matrix in both files and name the reference editor whose semantics are
matched.

## Testing conventions

- Pure helpers (word motions, kill ring, undo, CSI decode, …) are unit-tested
  in the `tests` module at the bottom of `src/lib.rs`.
- Terminal behavior is tested end-to-end in `tests/pty.rs`, which spawns the
  built `examples/demo` binary under a pseudo-terminal, feeds it keystrokes,
  and asserts on ANSI-stripped output. An executed line's signature is
  `\r\n<line>\r\n` (repaints never produce that). Keep escape sequences whole
  within a single input chunk. rush's downstream pty harness covers more
  scenarios.
- `#![warn(missing_docs)]` is on: new public items need doc comments.
