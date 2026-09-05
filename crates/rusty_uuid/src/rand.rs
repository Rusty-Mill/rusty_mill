//! OS-backed random byte source, via [`rusty_rand`] (a dependency-free
//! workspace sibling extracted from this module's own former copy and
//! `rusty_oauth`'s identical one): `/dev/urandom` on Unix behind a cached
//! handle, `BCryptGenRandom` on Windows.

/// Fills `buf` from the OS CSPRNG.
///
/// Panics if the OS cannot supply random bytes -- `Uuid::new_v4` is
/// infallible by design (matching every other UUID crate), and a machine
/// whose CSPRNG is unavailable has no safe way to mint a v4 UUID at all.
pub(crate) fn fill(buf: &mut [u8]) {
    if let Err(err) = rusty_rand::fill(buf) {
        panic!("rusty_uuid: {err}");
    }
}
