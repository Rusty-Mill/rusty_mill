//! `TrustAnchors` trait impl over the sys layer (rustils#88). No
//! `unsafe` here.
//!
//! The first `security` surface on this backend, which was net-only
//! until now (rustils#48/#86). It arrives for the same reason the net
//! slice did: a named consumer (`rusty_tls`) forced it, and only the
//! narrowest piece it needs — `Csprng`/`CredentialStore`/`Sandbox` stay
//! out of scope here until something forces them too (RFC v2 §3).

use platform::error::Result;
use platform::security::TrustAnchors;

use crate::sys::trust_anchors;

/// The BSD backend's [`TrustAnchors`] capability (rustils#88).
/// Stateless — every call re-reads, mirroring [`crate::BsdNet`]'s own
/// statelessness, so a machine whose trust store just changed doesn't
/// keep serving the old anchor set for the life of the process.
///
/// Two mechanisms behind one type: Security.framework on macOS,
/// PEM-file probing on FreeBSD/OpenBSD/NetBSD/DragonFly. See
/// [`crate::sys::trust_anchors`] for why that split is real here rather
/// than the portable-subset arrangement this crate's `net` slice uses.
///
/// Carries macOS's built-in-roots-only fidelity limit; see
/// [`TrustAnchors`].
pub struct BsdTrustAnchors;

impl TrustAnchors for BsdTrustAnchors {
    fn load_anchors(&self) -> Result<Vec<Vec<u8>>> {
        trust_anchors::load_anchors()
    }
}
