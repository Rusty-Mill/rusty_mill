//! Safe wrappers over the ffi layer. All `unsafe` in this crate lives in
//! this module tree, one documented invariant per block.

pub mod dbus;
pub mod fdio;
pub mod identity;
pub mod net;
pub mod pty;
pub mod secret_service;
pub mod security;
pub mod signals;
pub mod spawn;
pub mod termios;
pub mod trust_anchors;
pub mod tun;
