//! A zeroize-on-drop key storage and secure file persistence micro-crate.
//!
//! Protects sensitive cryptographic keys in memory by zeroizing memory allocations on `Drop`
//! using volatile writes, and provides platform-aware restricted file permissions (`0600` on Unix).

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use core::fmt;

/// Zero-on-drop secret byte vector wrapper.
pub struct SecretBytes {
    buf: Vec<u8>,
}

impl SecretBytes {
    /// Wrap an existing byte vector as a secret.
    pub fn new(buf: Vec<u8>) -> Self {
        SecretBytes { buf }
    }

    /// Borrow secret bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Length of secret key buffer.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` if empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        // Volatile zeroization to ensure compiler cannot optimize away memory wipe
        for byte in self.buf.iter_mut() {
            unsafe {
                core::ptr::write_volatile(byte, 0);
            }
        }
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes([REDACTED {} bytes])", self.buf.len())
    }
}

impl PartialEq for SecretBytes {
    fn eq(&self, other: &Self) -> bool {
        // Constant-time byte comparison to prevent timing side-channels
        if self.buf.len() != other.buf.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in self.buf.iter().zip(other.buf.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

impl Eq for SecretBytes {}

#[cfg(feature = "std")]
mod file {
    use super::SecretBytes;
    use std::io;
    use std::path::Path;

    impl SecretBytes {
        /// Persists this secret to `path`, creating (or truncating) it with
        /// permissions restricted to the owner only.
        ///
        /// On Unix this opens the file with mode `0600` from the moment of
        /// creation (`O_CREAT` and the mode are applied atomically by the
        /// OS, so there is no window where the file exists world-readable).
        /// **On Windows there is currently no equivalent ACL restriction**
        /// applied — the file inherits whatever permissions its containing
        /// directory grants. That gap is a known, documented limitation,
        /// not a silent claim of parity with the Unix behavior.
        pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
            use std::io::Write;
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options.open(path)?;
            f.write_all(&self.buf)?;
            f.flush()
        }

        /// Loads a secret previously written by [`Self::save_to_file`].
        /// Does not itself verify or tighten the file's permissions —
        /// callers on Windows should not rely on this path being
        /// access-restricted (see [`Self::save_to_file`]'s doc comment).
        pub fn load_from_file(path: &Path) -> io::Result<Self> {
            let buf = std::fs::read(path)?;
            Ok(SecretBytes { buf })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_bytes_redacts_debug() {
        let secret = SecretBytes::new(vec![1, 2, 3, 4, 5]);
        let debug_str = format!("{secret:?}");
        assert_eq!(debug_str, "SecretBytes([REDACTED 5 bytes])");
    }

    #[test]
    fn constant_time_eq() {
        let s1 = SecretBytes::new(vec![0xAA, 0xBB, 0xCC]);
        let s2 = SecretBytes::new(vec![0xAA, 0xBB, 0xCC]);
        let s3 = SecretBytes::new(vec![0xAA, 0xBB, 0xDD]);
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[cfg(feature = "std")]
    #[test]
    fn save_then_load_round_trips_real_file_contents() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rusty_crypto_key_test_{}.bin", std::process::id()));

        let secret = SecretBytes::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        secret.save_to_file(&path).expect("save should succeed");
        let loaded = SecretBytes::load_from_file(&path).expect("load should succeed");
        assert_eq!(secret, loaded);

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(all(feature = "std", unix))]
    #[test]
    fn save_to_file_restricts_permissions_to_owner_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rusty_crypto_key_test_perms_{}.bin",
            std::process::id()
        ));

        let secret = SecretBytes::new(vec![1, 2, 3]);
        secret.save_to_file(&path).expect("save should succeed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "file should be owner-only read/write");

        let _ = std::fs::remove_file(&path);
    }
}
