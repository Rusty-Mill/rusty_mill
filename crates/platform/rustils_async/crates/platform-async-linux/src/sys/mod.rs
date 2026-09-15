//! Safe wrappers over raw Linux syscalls. All `unsafe` in this crate
//! lives in this module tree, one documented invariant per block —
//! same discipline as rustils' own `platform-linux::sys`.

pub mod pidfd;
pub mod reactor;
