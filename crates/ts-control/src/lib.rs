//! ts2021 control-plane client: Noise IK channel, registration, netmap
//! long-poll. Protocol spec: PROTOCOL.md at the workspace root; ground truth
//! is Go `control/{controlbase,controlhttp,controlclient}` at v1.86.2.

mod base64;
mod client;
pub mod controlbase;
pub mod controlhttp;
mod prefixed;

pub use client::{ClientError, ControlClient};
pub use controlhttp::{ControlHttpError, ControlUrl};
