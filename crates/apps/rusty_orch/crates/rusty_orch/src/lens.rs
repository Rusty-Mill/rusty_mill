//! The `rusty_orch lens` shell (#594): everything the lens runner does that
//! touches a process, the network or the disk. Decisions stay in the pure
//! `orch-lens` core.
//!
//! - [`registry`]: one [`orch_lens::Model`] per configured provider, with
//!   the operator's assertions passed to the CLI adapters.

pub mod registry;
