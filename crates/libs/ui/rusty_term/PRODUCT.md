# Product

<!-- impeccable:product-schema 1 -->

Scope: the rusty_term windowed front-end (`--gui`) and the rusty_mux client it is becoming.
Not the rest of the rusty_mill monorepo.

## Platform

web

<!-- Impeccable has no "desktop" value. rusty_term is a native winit window (CPU softbuffer or
GPU wgpu), not a browser page; "web" here only means "not iOS/Android native guidance". -->

## Users

- Primary: Nano, the owner. Solo developer, Windows and PowerShell locally (hosts Lap and
  Beast), Linux remotely. A visual learner who wants small, terse, coherent tools.
- Secondary: AI coding agents running in their own blocks, launched by `bbp mod`, reporting
  status over OSC 7501. They are watched and steered from the GUI, and accept typing.

## Product Purpose

A terminal emulator written from scratch in Rust that the owner controls end to end, from the
Windows console host up to the window. It ends as one GUI for local terminals and rusty_mux
sessions: sessions hold windows, windows hold split trees of blocks, some blocks driven by
agents. Success: it is Nano's daily terminal on Windows, faster than the peers it measures
against, with every component owned in-repo.

## Positioning

- Owns the whole stack: its own console host (`rusty_conhost`, the Windows default) instead of
  ConPTY, its own async runtime (`rusty_tokio`), minimal third-party dependencies.
- The mux server owns the layout; the GUI only draws it, so any client sees the same session.
- Agent blocks are first class: status, lease and take-over live in the block header.

## Operating Context

- Daily shell work on Windows plus remote sessions over rusty_mux, where 50 to 200 ms of added
  latency is normal.
- Keyboard first: the Ctrl-] prefix for layout (`d` detach, hjkl/arrows focus, `z` zoom,
  `|` and `-` split, `x` close); Windows Terminal clipboard habits (Ctrl+Shift+V, Ctrl+V,
  Ctrl+C copies when there is a selection).
- First-run setup page: shell, theme, clipboard, alerts; Skip writes nothing.

## Capabilities and Constraints

- Terms: "block" means a layout pane and nothing else. "Chrome" is everything that is not
  program output.
- The chrome uses its own UI font at a fixed size times DPI; font size and zoom change only the
  terminal grid.
- Performance is a product feature: throughput floors and IR ceilings gate CI.
- Owner gates: new crates, names or dependencies; lockfile and toolchain changes; public API
  changes; each roadmap item needs its own go.
- Open: an explicit "create config template" action (handoff Q18).

## Brand Commitments

- Name: rusty_term (with rusty_mux). Quiet chrome; the terminal is the subject.
- Copy never overclaims; no "please", "simply" or "just"; an error names what failed and one
  action.
- Visual authority lives in `docs/projects/term/gui-design/` (design pack D1 to D36) and
  `docs/projects/term/STYLEGUIDE.md`. This file does not restate it.

## Evidence on Hand

- Design pack frames F1 to F8 in Paper plus `docs/projects/term/gui-design/assets/html/`.
- Benchmarks under `crates/libs/ui/rusty_term/bench/` and the roadmap's measured numbers.
- No users other than the owner, no testimonials, no pricing. Do not invent any.

## Product Principles

1. Own the stack: replace a component rather than argue it can't be replaced.
2. Small, terse, coherent: no speculative features; each item ships alone with its own go.
3. The terminal is the subject; chrome stays out of the way.
4. Keyboard first and remote-tolerant.
5. Measure before claiming speed.

## Accessibility & Inclusion

- Text contrast at least 4.5:1 (greyed-out suggestion text excepted).
- Keyboard focus always visible.
- Every animation has a reduced-motion version; under the OS setting, motion stops.
