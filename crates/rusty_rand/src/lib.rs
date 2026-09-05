//! OS-backed cryptographically secure random bytes, with no external
//! dependencies.
//!
//! Extracted from three near-identical copies -- `rusty_oauth::rand`,
//! `rusty_uuid`'s private `rand` module, and `sessionmgr-proc`'s
//! `os_random` -- which had each independently arrived at the same
//! two-backend design:
//!
//! - Unix: read from `/dev/urandom`, which never blocks once the kernel
//!   entropy pool is initialized (Linux 5.6+, macOS, the BSDs). The file
//!   handle is opened once and cached, as `rusty_uuid`'s copy did, so a
//!   caller minting many small values doesn't pay an `open(2)` each time.
//! - Windows: `BCryptGenRandom` with `BCRYPT_USE_SYSTEM_PREFERRED_RNG`
//!   from the CNG API, via a hand-declared FFI binding to `bcrypt.dll` --
//!   no `windows-sys`, no crate.
//!
//! Failures are returned, never papered over with a weaker source: a
//! caller deriving a PKCE verifier or a session id from these bytes must
//! find out if the OS could not supply them.
//!
//! Deliberately *not* the raw `getrandom(2)` syscall that `rusty_libc` and
//! `rustils`' `platform-linux` use. Those are Linux-only (or, for
//! `platform`, gated behind that project's own consumer policy), and this
//! crate has to serve macOS/BSD consumers through the same code path.

use std::fmt;

/// The OS refused or failed to supply random bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to obtain secure random bytes: {}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<Error> for std::io::Error {
    fn from(err: Error) -> Self {
        std::io::Error::other(err)
    }
}

/// Fills `buf` with cryptographically secure random bytes from the OS
/// CSPRNG. An empty `buf` is a no-op that never touches the OS.
pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
    if buf.is_empty() {
        return Ok(());
    }
    imp::fill(buf)
}

/// Returns `len` cryptographically secure random bytes.
pub fn bytes(len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; len];
    fill(&mut buf)?;
    Ok(buf)
}

#[cfg(unix)]
mod imp {
    use super::Error;
    use std::fs::File;
    use std::io::Read;
    use std::sync::{Mutex, OnceLock};

    static URANDOM: OnceLock<Mutex<File>> = OnceLock::new();

    /// The cached `/dev/urandom` handle, opened on first use. `OnceLock`
    /// can't run a fallible initializer, so the open happens outside it;
    /// two threads racing here both open the device and one handle wins,
    /// the other closing on drop -- harmless.
    fn urandom() -> Result<&'static Mutex<File>, Error> {
        if let Some(file) = URANDOM.get() {
            return Ok(file);
        }
        let file =
            File::open("/dev/urandom").map_err(|e| Error(format!("open /dev/urandom: {e}")))?;
        Ok(URANDOM.get_or_init(|| Mutex::new(file)))
    }

    pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
        let file = urandom()?;
        // A poisoned lock means another thread panicked mid-read; the
        // handle itself is still a valid, readable `/dev/urandom`.
        let mut guard = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .read_exact(buf)
            .map_err(|e| Error(format!("read /dev/urandom: {e}")))
    }
}

#[cfg(windows)]
mod imp {
    use super::Error;

    // Named to match the Windows API exactly, not Rust convention -- an
    // FFI binding is clearer when it reads the way the real API docs do.
    #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
    type NTSTATUS = i32;
    #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
    type ULONG = u32;
    #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
    type PUCHAR = *mut u8;
    #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
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

    pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
        // SAFETY: `BCryptGenRandom` with a null algorithm handle and
        // `BCRYPT_USE_SYSTEM_PREFERRED_RNG` writes exactly `cb_buffer`
        // bytes into `pb_buffer`, which is the caller's live, mutable,
        // `buf.len()`-byte slice.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as ULONG,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(Error(format!(
                "BCryptGenRandom failed with NTSTATUS 0x{status:08x}"
            )));
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::Error;

    pub fn fill(_buf: &mut [u8]) -> Result<(), Error> {
        Err(Error(
            "no supported OS CSPRNG source on this platform".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_requested_length() {
        let out = bytes(32).expect("random bytes");
        assert_eq!(out.len(), 32);
    }

    #[test]
    fn empty_buffer_is_a_no_op() {
        let mut empty: [u8; 0] = [];
        fill(&mut empty).expect("empty fill");
        assert!(bytes(0).expect("zero bytes").is_empty());
    }

    #[test]
    fn not_all_zero_and_differs_between_calls() {
        let a = bytes(32).expect("a");
        let b = bytes(32).expect("b");
        assert_ne!(a, [0u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn concurrent_callers_share_the_cached_source() {
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| bytes(64).expect("random bytes")))
            .collect();
        let outputs: Vec<Vec<u8>> = handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect();
        for (i, a) in outputs.iter().enumerate() {
            for b in &outputs[i + 1..] {
                assert_ne!(a, b, "two threads got identical 64-byte outputs");
            }
        }
    }

    #[test]
    fn error_display_and_io_conversion() {
        let err = Error("simulated".to_string());
        assert_eq!(
            err.to_string(),
            "failed to obtain secure random bytes: simulated"
        );
        let io: std::io::Error = err.into();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
    }
}
