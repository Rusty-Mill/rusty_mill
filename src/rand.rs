//! Cryptographically secure random byte generation, with zero external
//! dependencies. Used for PKCE code verifiers (RFC 7636), `state` and
//! `nonce` values (RFC 6749 §10.12 / OIDC), and any other value that must
//! be unguessable.
//!
//! Sourced directly from the operating system's CSPRNG:
//! - Unix: reads from `/dev/urandom`.
//! - Windows: calls `BCryptGenRandom` via a manually declared FFI binding
//!   to `bcrypt.dll` (part of the Windows CNG API) — no crate required.

use std::fmt;

#[derive(Debug)]
pub struct RandError(String);

impl fmt::Display for RandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to obtain secure random bytes: {}", self.0)
    }
}

impl std::error::Error for RandError {}

/// Fills `buf` with cryptographically secure random bytes from the OS CSPRNG.
pub fn fill_random(buf: &mut [u8]) -> Result<(), RandError> {
    imp::fill_random(buf)
}

/// Returns `len` cryptographically secure random bytes.
pub fn random_bytes(len: usize) -> Result<Vec<u8>, RandError> {
    let mut buf = vec![0u8; len];
    fill_random(&mut buf)?;
    Ok(buf)
}

#[cfg(unix)]
mod imp {
    use super::RandError;
    use std::fs::File;
    use std::io::Read;

    pub fn fill_random(buf: &mut [u8]) -> Result<(), RandError> {
        let mut file =
            File::open("/dev/urandom").map_err(|e| RandError(format!("open /dev/urandom: {e}")))?;
        file.read_exact(buf)
            .map_err(|e| RandError(format!("read /dev/urandom: {e}")))?;
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::RandError;

    #[allow(non_camel_case_types)]
    type NTSTATUS = i32;
    #[allow(non_camel_case_types)]
    type ULONG = u32;
    #[allow(non_camel_case_types)]
    type PUCHAR = *mut u8;
    #[allow(non_camel_case_types)]
    type PVOID = *mut core::ffi::c_void;

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: ULONG = 0x0000_0002;

    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(
            h_algorithm: PVOID,
            pb_buffer: PUCHAR,
            cb_buffer: ULONG,
            dw_flags: ULONG,
        ) -> NTSTATUS;
    }

    pub fn fill_random(buf: &mut [u8]) -> Result<(), RandError> {
        if buf.is_empty() {
            return Ok(());
        }
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as ULONG,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(RandError(format!("BCryptGenRandom failed: 0x{status:08x}")));
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::RandError;

    pub fn fill_random(_buf: &mut [u8]) -> Result<(), RandError> {
        Err(RandError(
            "no supported OS CSPRNG source on this platform".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_requested_length() {
        let bytes = random_bytes(32).expect("random bytes");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn not_all_zero_and_differs_between_calls() {
        let a = random_bytes(32).unwrap();
        let b = random_bytes(32).unwrap();
        assert_ne!(a, [0u8; 32]);
        assert_ne!(a, b);
    }
}
