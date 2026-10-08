//! HKDF (RFC 5869) over any [`Hash`].

use crate::{Hash, Hmac};
use core::fmt;
use rusty_crypto_key::wipe;

/// Requested output longer than `255 * hash length`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpandError;

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HKDF output length exceeds 255 hash blocks")
    }
}

/// A pseudorandom key, the output of [`extract`]. Wiped on drop.
pub struct Prk<H: Hash>(H::Output);

/// `HKDF-Extract(salt, ikm)`. An absent salt is a string of zeros of the hash
/// length, as RFC 5869 section 2.2 specifies.
pub fn extract<H: Hash>(salt: Option<&[u8]>, ikm: &[u8]) -> Prk<H> {
    let zeros = [0u8; 64];
    let salt = salt.unwrap_or(&zeros[..H::OUTPUT_LEN]);
    Prk(Hmac::<H>::mac(salt, ikm))
}

impl<H: Hash> Prk<H> {
    /// Uses `bytes` directly as the pseudorandom key (TLS 1.3 secrets are
    /// already one). Returns `None` unless it is exactly the hash length: RFC
    /// 5869 section 2.3 forbids shorter, and a longer one is only ever a bug.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != H::OUTPUT_LEN {
            return None;
        }
        let mut prk = H::new().finalize();
        prk.as_mut().copy_from_slice(bytes);
        Some(Self(prk))
    }

    /// `HKDF-Expand(prk, info, out.len())`, filling `out`.
    pub fn expand(&self, info: &[u8], out: &mut [u8]) -> Result<(), ExpandError> {
        if out.len() > 255 * H::OUTPUT_LEN {
            return Err(ExpandError);
        }
        let mut previous = H::new().finalize();
        let mut have_previous = false;
        for (index, chunk) in out.chunks_mut(H::OUTPUT_LEN).enumerate() {
            let mut mac = Hmac::<H>::new(self.0.as_ref());
            if have_previous {
                mac.update(previous.as_ref());
            }
            mac.update(info);
            // `index < 255` was checked above, so the counter fits.
            mac.update(&[(index + 1) as u8]);
            wipe(previous.as_mut());
            previous = mac.finalize();
            have_previous = true;
            chunk.copy_from_slice(&previous.as_ref()[..chunk.len()]);
        }
        wipe(previous.as_mut());
        Ok(())
    }
}

impl<H: Hash> Drop for Prk<H> {
    fn drop(&mut self) {
        wipe(self.0.as_mut());
    }
}
