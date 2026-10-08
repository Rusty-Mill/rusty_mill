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

/// Compares two byte strings without exiting early on the first difference.
///
/// Use it for secrets and MACs (bearer tokens, signatures, PKCE verifiers):
/// `==` short-circuits, so response latency would reveal how many leading
/// bytes a guess got right. The length is folded into the same accumulator and
/// the shorter input is zero-padded, so a wrong-length guess costs the same as
/// a wrong-value one. `black_box` stops the optimiser from reintroducing an
/// early exit it could prove equivalent for a boolean result.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    core::hint::black_box(diff) == 0
}

/// Overwrites `buf` with `T::default()` using volatile writes, so the compiler
/// cannot drop the wipe as a dead store. For stack and array secrets that
/// [`SecretBytes`] (heap only) does not cover.
pub fn wipe<T: Copy + Default>(buf: &mut [T]) {
    for item in buf.iter_mut() {
        // SAFETY: `item` is a valid, aligned, exclusive reference.
        unsafe {
            core::ptr::write_volatile(item, T::default());
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

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
        constant_time_eq(&self.buf, &other.buf)
    }
}

impl Eq for SecretBytes {}

#[cfg(feature = "std")]
mod file {
    use super::SecretBytes;
    use std::io;
    use std::path::Path;

    impl SecretBytes {
        /// Persists this secret to `path`, creating or replacing it
        /// crash-atomically (`rusty_atomic_file::write_private`).
        ///
        /// On Unix the new file is `0600` from the moment it is created, so
        /// the secret is never group/world-readable. That holds even when
        /// `path` already existed with looser permissions, since the file
        /// is replaced rather than truncated in place. If `path` is a
        /// symlink, this refuses with an error rather than replacing the
        /// link.
        ///
        /// **On Windows there is currently no ACL restriction applied**:
        /// the file inherits whatever permissions its containing directory
        /// grants. That gap is a known, documented limitation, not a
        /// silent claim of parity with the Unix behavior.
        ///
        /// # Errors
        /// `AlreadyExists` if `path` is a symlink; otherwise any I/O error
        /// from the write, which leaves `path` unchanged.
        pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
            if let Ok(meta) = std::fs::symlink_metadata(path) {
                if meta.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "refusing to save secret through a symlink",
                    ));
                }
            }
            rusty_atomic_file::write_private(path, &self.buf)
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
    #[test]
    fn wipe_zeroes_bytes_and_words() {
        let mut bytes = [0xffu8; 5];
        super::wipe(&mut bytes);
        assert_eq!(bytes, [0; 5]);
        let mut words = [u64::MAX; 3];
        super::wipe(&mut words);
        assert_eq!(words, [0; 3]);
        super::wipe::<u8>(&mut []);
    }

    use super::*;

    #[test]
    fn secret_bytes_redacts_debug() {
        let secret = SecretBytes::new(vec![1, 2, 3, 4, 5]);
        let debug_str = format!("{secret:?}");
        assert_eq!(debug_str, "SecretBytes([REDACTED 5 bytes])");
    }

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"ab", b"abc"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn constant_time_eq_rejects_zero_padded_prefix() {
        // The padded tail must not make a shorter input equal a longer one.
        assert!(!constant_time_eq(&[1], &[1, 0]));
        assert!(!constant_time_eq(&[1, 0, 0], &[1]));
    }

    #[test]
    fn secret_bytes_equality_is_byte_equality() {
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

    #[cfg(all(feature = "std", unix))]
    #[test]
    fn save_to_file_tightens_permissions_on_preexisting_readable_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rusty_crypto_key_test_preexisting_{}.bin",
            std::process::id()
        ));

        // Pre-create the target with loose (world/group-readable)
        // permissions and placeholder content, as if it had been written
        // by some other, less careful process.
        std::fs::write(&path, b"placeholder").expect("setup write should succeed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("setup chmod should succeed");

        let secret = SecretBytes::new(vec![9, 8, 7, 6, 5]);
        secret.save_to_file(&path).expect("save should succeed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "save_to_file must tighten permissions of a pre-existing readable file"
        );

        let loaded = SecretBytes::load_from_file(&path).expect("load should succeed");
        assert_eq!(secret, loaded);

        let _ = std::fs::remove_file(&path);
    }
}
