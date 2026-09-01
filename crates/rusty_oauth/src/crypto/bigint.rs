//! Re-exports the shared `BigUint` type from `rusty_rsa`, which also
//! backs `rusty_rdp`'s independent RSA implementation. The two crates'
//! hand-rolled bignum arithmetic was near-identical duplication --
//! extracted into `rusty_rsa` (see that crate's own docs for the
//! mechanism/policy split behind what moved and what didn't).

pub use rusty_rsa::BigUint;
