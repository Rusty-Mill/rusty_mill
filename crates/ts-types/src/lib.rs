//! Core wire and API types shared across the tailscale-rs workspace.
//!
//! Phase 1 scope: the LocalAPI surface — node/peer status (`ipnstate`),
//! preferences (`prefs`), ping results, and the typed key/ID/address
//! primitives they are built from. Netmap and DERP-map types arrive with
//! Phase 2.
//!
//! Ground truth is the Go implementation (`ipn/ipnstate/ipnstate.go`,
//! `ipn/prefs.go`, `tailcfg/tailcfg.go` at v1.86); golden fixtures captured
//! from a live tailscaled 1.86.2 pin the JSON encoding (see `tests/`).
//!
//! Decoding rules (see DESIGN.md): unknown fields are ignored, missing
//! fields default, and JSON `null` on collection fields decodes as empty —
//! the Go structs gain and omit fields across versions and this crate must
//! stay compatible with the binaries we interop with.

#![forbid(unsafe_code)]

mod hex;
mod ids;
mod ipnstate;
mod key;
mod net;
mod prefs;
mod time;

pub use ids::{StableNodeID, UserID};
pub use ipnstate::{ExitNodeStatus, PeerStatus, PingResult, Status, TailnetStatus, UserProfile};
pub use key::{DiscoPublic, KeyParseError, MachinePublic, NodePublic};
pub use net::{IpPrefix, PrefixParseError};
pub use prefs::{MaskedPrefs, Prefs};
pub use time::Rfc3339;

/// Deserializes JSON `null` as `T::default()`.
///
/// Go marshals nil slices and maps as `null` (e.g. `"Addrs": null`); plain
/// `#[serde(default)]` only covers *absent* fields, so nullable collections
/// use `#[serde(default, deserialize_with = "null_default")]`.
pub(crate) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    let opt = <Option<T> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}
