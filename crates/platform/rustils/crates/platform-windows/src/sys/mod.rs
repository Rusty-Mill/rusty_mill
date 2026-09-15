//! Safe wrappers over the ffi layer. All `unsafe` in this crate lives in
//! this module tree, one documented invariant per block.

pub mod console;
pub mod csignals;
pub mod errmap;
pub mod fileio;
pub mod handle;
pub mod net;
pub mod nt;
pub mod proc;
pub mod pty;
pub mod security;
