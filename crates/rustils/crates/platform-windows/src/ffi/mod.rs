//! Curated windows-sys import surface — the exact APIs this backend may
//! touch. Widening this list is a reviewed decision (same discipline as
//! the Linux backend's `libc_surface`).
//!
//! Scope note (D-15): this curates *windows-sys*, which is the default
//! floor and the only one under `#[cfg(not(feature = "track-w"))]`. The
//! `track-w` arms call `rusty_win32` directly rather than through a
//! surface module here, matching how `platform-linux` treats `rusty_libc`
//! under `track-p` — a curation point exists to narrow a large
//! machine-generated crate down to a reviewable list, and neither donor
//! crate needs that (each is a hand-written, already-reviewed surface,
//! and the migration inventory lives in `docs/convergence-roadmap.md`
//! §1d instead). So this file stays the honest checklist for the
//! configuration that ships by default, not a complete inventory of every
//! foreign call reachable under every feature combination.

pub mod nt_surface;
pub mod win32_surface;
