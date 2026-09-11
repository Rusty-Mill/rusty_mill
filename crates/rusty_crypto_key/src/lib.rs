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
        /// Persists this secret to `path`, creating (or replacing) it with
        /// permissions restricted to the owner only.
        ///
        /// On Unix this writes the secret to a freshly-created sibling temp
        /// file (mode `0600` from the moment of creation, so it is never
        /// world/group-readable) and then atomically renames it over
        /// `path`. This guarantees restrictive permissions on the final
        /// file even when `path` already exists and was previously created
        /// with looser permissions — opening an existing file with a
        /// creation-only mode does not retroactively tighten it, which is
        /// why a rename-based swap is used instead of truncating in place.
        /// If `path` already exists as a symlink, this refuses to follow
        /// it and returns an error, rather than silently replacing
        /// whatever the link points at.
        ///
        /// **On Windows there is currently no equivalent ACL restriction**
        /// applied — the file inherits whatever permissions its containing
        /// directory grants. That gap is a known, documented limitation,
        /// not a silent claim of parity with the Unix behavior.
        #[cfg(unix)]
        pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            use std::sync::atomic::{AtomicU64, Ordering};

            // Refuse to write through a pre-existing symlink: silently
            // following it could redirect the secret to an
            // attacker-controlled location outside the intended directory.
            if let Ok(meta) = std::fs::symlink_metadata(path) {
                if meta.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "refusing to save secret through a symlink",
                    ));
                }
            }

            let dir = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let file_name = path.file_name().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "path has no file name")
            })?;

            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let pid = std::process::id();

            let (mut tmp_file, tmp_path) = loop {
                let n = COUNTER.fetch_add(1, Ordering::Relaxed);
                let mut tmp_name = file_name.to_os_string();
                tmp_name.push(format!(".{pid}.{n}.tmp"));
                let tmp_path = dir.join(&tmp_name);
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&tmp_path)
                {
                    Ok(f) => break (f, tmp_path),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e),
                }
            };

            let write_result = tmp_file.write_all(&self.buf).and_then(|_| tmp_file.flush());
            drop(tmp_file);
            if let Err(e) = write_result {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e);
            }

            if let Err(e) = std::fs::rename(&tmp_path, path) {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e);
            }

            Ok(())
        }

        /// Persists this secret to `path`, creating (or truncating) it.
        ///
        /// **On Windows there is currently no ACL restriction applied** —
        /// the file inherits whatever permissions its containing directory
        /// grants. That gap is a known, documented limitation, not a
        /// silent claim of parity with the Unix behavior (see the Unix
        /// implementation of this method for the restricted-permissions
        /// guarantee it provides).
        #[cfg(not(unix))]
        pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
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
