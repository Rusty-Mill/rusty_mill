//! # platform-linux — the Linux backend
//!
//! Layering (RFC v2 §4.1): `ffi` (raw bindings — currently the `libc`
//! crate, curated in `ffi::libc_surface`) → `sys` (safe wrappers; **all
//! `unsafe` in this crate lives there**, each block with a documented
//! invariant) → the trait impls at the crate root, which contain no
//! `unsafe`.
//!
//! Tier doctrine (RFC v2 §2, decision D-2): the floor is libc for now.
//! Track P — replacing libc call-by-call with raw syscalls behind a
//! feature — is deliberately absent until Release R2 has shipped; a
//! learning track must never block a consumer.

#![cfg(target_os = "linux")]
#![deny(unsafe_code)] // opted back in, narrowly, inside sys/ modules only

pub mod ffi;
pub mod sys;

mod fs;
mod net;
mod process;
mod pty;
mod security;
mod signals;
mod term;
mod tun;

pub use fs::{LinuxAnonymousFile, LinuxDir, LinuxFile};
pub use net::{
    LinuxNet, LinuxTcpListener, LinuxTcpStream, LinuxUdpSocket, LinuxUnixListener, LinuxUnixStream,
};
pub use process::{LinuxChild, LinuxSpawner};
pub use pty::{LinuxPty, LinuxPtyMaster};
pub use security::{LinuxCredentialStore, LinuxCsprng, LinuxSandbox, LinuxTrustAnchors};
pub use signals::LinuxSignalSource;
pub use sys::identity::{group_name, user_name};
pub use term::LinuxTerminal;
pub use tun::{LinuxTun, LinuxTunDevice};
