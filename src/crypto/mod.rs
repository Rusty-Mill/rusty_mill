//! Hand-rolled cryptographic primitives, std-only.
//!
//! RDP **standard security** (MS-RDPBCGR §5.3) is built on a specific and now
//! obsolete set of primitives — MD5, SHA-1, RC4, and RSA public-key
//! encryption. RDP's **Network Level Authentication** (NLA / CredSSP) adds a
//! few more: MD4 and HMAC-MD5 for NTLMv2, and SHA-256 for the CredSSP public-
//! key channel binding. To keep the crate dependency-free they are all
//! implemented here by hand rather than pulled from a crypto crate.
//!
//! ## Security warning
//!
//! **Most of these are not safe for new designs.** MD4, MD5, and SHA-1 are
//! broken as collision-resistant hashes, RC4 is a broken cipher, NTLM is a
//! weak authentication protocol, and this RSA does no padding checks or
//! constant-time work. They exist only to speak the RDP wire protocols
//! (standard security and NTLM-based CredSSP). SHA-256 is not broken but is
//! still hand-rolled here to avoid a dependency. The TLS bytes themselves are
//! left to a vetted stack (the optional `tls` feature).

pub mod bignum;
pub mod hmac;
pub mod md4;
pub mod md5;
pub mod rc4;
pub mod sha1;
pub mod sha256;

pub use bignum::BigUint;
pub use hmac::hmac_md5;
pub use md4::md4;
pub use md5::{md5, Md5};
pub use rc4::Rc4;
pub use sha1::{sha1, Sha1};
pub use sha256::sha256;
