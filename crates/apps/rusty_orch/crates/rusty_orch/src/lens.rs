//! The `rusty_orch lens` shell (#594): everything the lens runner does that
//! touches a process, the network or the disk. Decisions stay in the pure
//! `orch-lens` core.
//!
//! - [`registry`]: one [`orch_lens::Model`] per configured provider, with
//!   the operator's assertions passed to the CLI adapters.
//! - [`assertions`]: the operator's assertions file, read fail-closed.
//! - [`bbp`]: one `bbp mcp` session per lens, its view of the board and
//!   its way to post.
//! - [`check`]: `rusty_orch lens check`, a dry run of the above.
//! - [`run`]: `rusty_orch lens run`, passes over every lens until quiet.
//! - [`container`]: the dedicated container a host-cleared provider runs in.
//! - [`alien`]: `rusty_orch lens alien report`, the #708 alien lens's shadow numbers.
//! - [`probes`]: the #708 alien lens's sandboxed probe runner (D96).
//! - [`hunts`]: the #708 alien lens's pass in `lens run --alien` (D99).

pub mod alien;
pub mod assertions;
pub mod bbp;
pub mod check;
pub mod container;
pub mod hunts;
pub mod probes;
pub mod registry;
pub mod run;
