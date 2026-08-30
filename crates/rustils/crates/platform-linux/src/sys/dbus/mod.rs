//! A hand-rolled D-Bus client (RFC v2 R5+, D15, Phase 6 item 2,
//! rustils#77): connection transport plus enough of the wire protocol
//! (message framing, the type-system marshaling rules) to make method
//! calls over the session bus. No existing D-Bus dependency — built from
//! scratch, matching this repo's raw-bindings philosophy over pulling in
//! a crate the way nexus's donor `keyring-rs` wrapper does.
//!
//! This module is deliberately *not* Secret Service-specific — it's
//! transport plumbing only, with no `CredentialStore` behavior wired up
//! yet (that's rustils#78, built on top of this). It's usable (and
//! tested) against any D-Bus service already, not just
//! `org.freedesktop.secrets`.
//!
//! Only the subset of the D-Bus type system Secret Service actually
//! needs is modeled: `y b n q i u x t d s o g a ( ) v { }` (every basic
//! type plus array/struct/variant/dict-entry). `h` (UNIX_FD) is not
//! modeled — no consumer here passes file descriptors over D-Bus.
//!
//! Only little-endian (`'l'`) messages are supported, both sent and
//! received — every real system this backend targets is little-endian,
//! and the D-Bus spec lets either side of a connection choose its own
//! messages' byte order, so a same-host session bus daemon on a
//! practically-always-LE Linux host has no reason to ever send `'B'`
//! (big-endian) messages back. See `value.rs`'s module doc comment for
//! the type-system detail and `transport.rs` for the session-bus
//! connection sequence (address discovery, `AF_UNIX` connect, SASL
//! `EXTERNAL` handshake).

mod message;
mod transport;
mod value;
mod wire;

pub use message::Message;
pub use transport::Connection;
pub use value::Value;
