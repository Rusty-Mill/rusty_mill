# Architecture

## Overview
`rusty_gpu` maintains an in-memory pixel buffer (`Framebuffer`), rasterizes
shapes into it via a `Pipeline`, and presents the finished buffer to an OS
window — a software (CPU-driven) renderer, not a wrapper over a GPU API like
Vulkan or DirectX. It's not a windowing toolkit or an OS abstraction itself;
window creation and the surface handle come from `rusty_gui`.

## Boundaries
Domain logic (`Color`, `Framebuffer` pixel operations, `Pipeline`
rasterization) is `#![no_std]` and platform-independent. The one boundary
that currently varies by OS is presentation — blitting the finished buffer to
a real window surface:

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `Framebuffer::present` | `rusty_win32::windowing::blit_pixel_buffer` (Windows, `cfg(windows)`) | No non-Windows adapter yet — `present` is a no-op off Windows. Depends on `rusty_gui::Window` for the target surface handle. |

## Structure
Single-crate modular monolith — `color`, `framebuffer`, `pipeline` are
submodules of one crate, not separate services. Ports-and-adapters keeps
rasterization logic (`Pipeline`) free of the presentation adapter's OS calls;
`Pipeline` only ever touches `Framebuffer`'s pixel-level API, never the
windowing/blit layer directly. No forcing function (independent scaling, a
team/language boundary, hard fault isolation) has come up yet to justify
splitting further.

## Data flow
1. Caller creates a `Framebuffer::new(width, height)`.
2. `Pipeline` methods (e.g. `draw_rect`) write `Color` values into the
   framebuffer via `set_pixel`.
3. Caller calls `Framebuffer::present(&window)` to blit the buffer to the OS
   window surface.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs. Sibling crates are pinned via `git` (not path) dependencies in
`Cargo.toml` — see the comment there and `rusty_tokio#254` — so this repo
builds standalone for any external consumer.

## Non-goals
- Not a GPU API wrapper (no Vulkan/DirectX/Metal backend) — CPU rasterization
  only. `rusty_vulkan` is the separate crate for that.
- Not a windowing toolkit — window creation/events come from `rusty_gui`.
