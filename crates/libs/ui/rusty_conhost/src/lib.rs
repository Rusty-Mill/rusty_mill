//! Our own Windows console host (design `docs/projects/term/conhost/`).
//!
//! A console client (`cmd.exe`, `pwsh`, any `.exe`) talks to ConDrv; we
//! serve its packets and turn console calls into VT for the terminal.
//! The crate has two halves:
//!
//! - a sans-I/O core (`console`, `input`, `cooked`, `ctrl`, `msg`,
//!   `sched`) that builds and tests on every platform, and
//! - on Windows, an engine thread over `platform_windows::condrv` and the
//!   pty-shaped front end [`OwnPty`] / [`OwnPtyMaster`].
//!
//! It serves output, input, the cursor and attribute APIs, and resize;
//! design §10 lists what is still to come.

// Off Windows nothing drives the core but its tests.
#[cfg_attr(not(windows), allow(dead_code))]
mod console;
#[cfg_attr(not(windows), allow(dead_code))]
mod cooked;
#[cfg_attr(not(windows), allow(dead_code))]
mod ctrl;
#[cfg_attr(not(windows), allow(dead_code))]
mod diag;
#[cfg_attr(not(windows), allow(dead_code))]
mod fixture;
#[cfg_attr(not(windows), allow(dead_code))]
mod input;
#[cfg_attr(not(windows), allow(dead_code))]
mod msg;
#[cfg_attr(not(windows), allow(dead_code))]
mod sched;

#[cfg(windows)]
mod pty;
#[cfg(windows)]
mod runtime;

#[cfg(windows)]
pub use pty::{ExitWaiter, OwnPty, OwnPtyMaster};
