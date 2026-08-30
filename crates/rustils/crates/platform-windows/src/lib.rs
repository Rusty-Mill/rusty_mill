//! # platform-windows — the Windows backend
//!
//! Layering (RFC v2 §4.1): `ffi` (curated windows-sys surface) → `sys`
//! (safe wrappers; all `unsafe` lives there with documented invariants) →
//! trait impls at the crate root.
//!
//! Tier doctrine (RFC v2 §2, decision D-1): `windows-sys` *is* the raw
//! floor on Windows — metadata-generated bindings are machine-known facts;
//! the hand-rolled value of this crate begins above them: typed handles,
//! lifetimes, error mapping, and (post-R2-hoist) the `winargv` quoting
//! module, which is this crate's security boundary.
//!
//! ## Track W (D-15): the `track-w` feature
//!
//! Off by default. When on, curated call families route through
//! `rusty_win32`'s hand-written `extern "system"` declarations instead of
//! windows-sys — a rev-pinned dependency migrated call-by-call, the same
//! shape `platform-linux` uses for `rusty_libc` behind `track-p`. Landed
//! families: `sys::fileio::read`/`write`.
//!
//! This does **not** revise D-1, and the symmetric feature name should not
//! be read as claiming it does. Track P descends a tier — raw syscalls
//! under libc, with the kernel ABI as the new floor. Track W cannot:
//! Windows publishes no supported tier beneath a documented DLL export
//! (the `ntdll` stubs are renumbered between builds on purpose), so both
//! configurations reach the identical `kernel32!ReadFile`. What the
//! feature swaps is the binding's *provenance* — hand-written and
//! reviewed, no `windows-targets` import-lib machinery, `no_std`-capable —
//! not its depth. Families rusty_win32 has no binding for at the pinned
//! rev stay on windows-sys in both configurations; [`ffi::nt_surface`]'s
//! whole `NtCreateFile` capability model is the largest of them.
//!
//! Both arms produce bit-identical [`platform::error::PlatformError`]s
//! (same classification table, same `OsCode::Win32`), which is what lets
//! this crate's entire suite re-run under `--features track-w` as the
//! equivalence test. See `docs/learning/003-…` for the full write-up,
//! including the error-path lesson: with a wrapper between caller and
//! call, the thread-local last-error slot is never the authority.
//!
//! ## Status: Dir/File landed (R1); winargv landed (R2 extraction step 1)
//!
//! The `Dir`/`File` impls run over `NtCreateFile` handle-relative opens
//! (`sys::nt`; the admission rationale lives in `ffi::nt_surface`) with
//! Win32 handle-based APIs for everything after the open. Developed from a
//! Linux host against `cargo check --target x86_64-pc-windows-gnu`; CI's
//! Windows leg is where the OS-touching tests actually run.
//!
//! The backend modules are `cfg(windows)`-gated individually rather than
//! at the crate root. [`winargv`] is re-exported from its own standalone
//! crate (convergence roadmap Phase 1c) rather than defined here: it is
//! pure string logic with no OS calls, and a handback consumer (rush,
//! rusty_naner) has no reason to depend on this crate's Windows-only
//! Dir/Spawner/console modules just for command-line quoting. It still
//! builds and tests on every host, under the Linux CI leg and Miri as
//! well as the Windows leg (its oracle test against
//! `CommandLineToArgvW` remains Windows-only, here in this crate's
//! `tests/`).

#![deny(unsafe_code)] // opted back in, narrowly, inside sys/ modules only

#[cfg(windows)]
pub mod ffi;
#[cfg(windows)]
pub mod fs;
#[cfg(windows)]
mod net;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod pty;
#[cfg(windows)]
mod security;
#[cfg(windows)]
mod signals;
#[cfg(windows)]
pub mod sys;
#[cfg(windows)]
mod term;
#[cfg(windows)]
mod tun;
#[cfg(windows)]
pub mod util;
pub use winargv;

#[cfg(windows)]
pub use fs::{WindowsAnonymousFile, WindowsDir, WindowsFile};
#[cfg(windows)]
pub use net::{
    WindowsNet, WindowsTcpListener, WindowsTcpStream, WindowsUdpSocket, WindowsUnixListener,
    WindowsUnixStream,
};
#[cfg(windows)]
pub use process::{WindowsChild, WindowsSpawner};
#[cfg(windows)]
pub use pty::{WindowsPty, WindowsPtyMaster};
#[cfg(windows)]
pub use security::{WindowsCredentialStore, WindowsCsprng, WindowsSandbox, WindowsTrustAnchors};
#[cfg(windows)]
pub use signals::WindowsSignalSource;
#[cfg(windows)]
pub use term::WindowsTerminal;
#[cfg(windows)]
pub use tun::WindowsTun;
