//! Safe wrappers over the curated `ffi` surface. All `unsafe` in this
//! crate lives in this module tree (RFC v2 §6), mirroring
//! `platform-linux::sys`'s layering.

pub mod fdio;
pub mod net;
