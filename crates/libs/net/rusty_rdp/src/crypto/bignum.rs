//! Re-exports the shared `BigUint` type from `rusty_rsa`, which also
//! backs `rusty_oauth`'s independent RSA implementation. The two crates'
//! hand-rolled bignum arithmetic was near-identical duplication --
//! extracted into `rusty_rsa` (see that crate's own docs for the
//! mechanism/policy split behind what moved and what didn't). This
//! crate's own RSA public/private key wrappers ([`RsaPublicKey`],
//! [`RsaPrivateKey`](crate::security::RsaPrivateKey) in
//! [`crate::security`]) stay local -- they wrap little-endian bytes for
//! RDP's own wire format, unlike `rusty_oauth`'s big-endian JWK-sourced
//! keys.

pub use rusty_rsa::BigUint;
