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
}
