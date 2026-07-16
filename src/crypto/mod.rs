//! Hand-rolled cryptographic primitives, std-only.
//!
//! RDP **standard security** (MS-RDPBCGR §5.3) is built on a specific and now
//! obsolete set of primitives — MD5, SHA-1, RC4, and RSA public-key
//! encryption. To keep the crate dependency-free they are implemented here by
//! hand rather than pulled from a crypto crate.
//!
//! ## Security warning
//!
//! **None of these are safe for new designs.** MD5 and SHA-1 are broken as
//! collision-resistant hashes, RC4 is a broken cipher, and this RSA does no
//! padding checks or constant-time work. They exist only to speak the RDP
//! standard-security wire protocol. Modern RDP deployments should prefer the
//! TLS/CredSSP security modes (the `SSL`/`HYBRID` negotiation), which will use
//! a vetted TLS stack rather than any of this.

pub mod bignum;
pub mod md5;
pub mod rc4;
pub mod sha1;

pub use bignum::BigUint;
pub use md5::{md5, Md5};
pub use rc4::Rc4;
pub use sha1::{sha1, Sha1};
