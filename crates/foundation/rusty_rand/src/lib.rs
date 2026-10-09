//! OS-backed cryptographically secure random bytes, with no external
//! dependencies.
//!
//! Extracted from three near-identical copies -- `rusty_oauth::rand`,
//! `rusty_uuid`'s private `rand` module, and `sessionmgr-proc`'s
//! `os_random` -- which had each independently arrived at the same
//! two-backend design:
//!
//! - Linux on x86_64/aarch64: `getrandom(2)` with flags 0 (via `rusty_libc`), which
//!   blocks until the kernel pool is seeded; `/dev/urandom` is the fallback
//!   for `ENOSYS` (kernel before 3.17).
//! - Other Unix (macOS, the BSDs, other Linux targets): read from `/dev/urandom`,
//!   which never blocks once the kernel entropy pool is initialized. The file
//!   handle is opened once and cached, so a caller minting many small values
//!   doesn't pay an `open(2)` each time; reads take no lock.
//! - Windows: `BCryptGenRandom` with `BCRYPT_USE_SYSTEM_PREFERRED_RNG`
//!   from the CNG API, via a hand-declared FFI binding to `bcrypt.dll` --
//!   no `windows-sys`, no crate.
//!
//! Failures are returned, never papered over with a weaker source: a
//! caller deriving a PKCE verifier or a session id from these bytes must
//! find out if the OS could not supply them.
//!
//! The kernel holds all the state, so there is no userspace generator to reseed after a
//! `fork`. Randomness *quality* is the OS's; this crate does not and cannot test it.

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
/// CSPRNG. An empty `buf` is a no-op that never touches the OS. On `Err` the
/// contents of `buf` are unspecified (possibly partly filled): do not use it.
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

/// Calls `read` until `buf` is full. `read` fills a prefix of the slice it is
/// given and returns its length. Interrupted calls are retried; a read that
/// makes no progress, claims more than it was given, or fails is an error, so a
/// short or empty read can never leave an unfilled tail that reads as random.
#[cfg_attr(not(unix), allow(dead_code))]
fn fill_with(
    buf: &mut [u8],
    mut read: impl FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Result<(), Error> {
    let mut done = 0;
    while done < buf.len() {
        let rest = &mut buf[done..];
        let cap = rest.len();
        match read(rest) {
            Ok(0) => return Err(Error("the source returned no bytes".to_string())),
            Ok(n) if n > cap => {
                return Err(Error(format!("the source claimed {n} of {cap} bytes")))
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error(e.to_string())),
        }
    }
    Ok(())
}

#[cfg(unix)]
mod imp {
    use super::{fill_with, Error};
    use std::fs::File;
    use std::io::Read;
    use std::sync::OnceLock;

    static URANDOM: OnceLock<File> = OnceLock::new();

    /// The cached `/dev/urandom` handle, opened on first use. `OnceLock`
    /// can't run a fallible initializer, so the open happens outside it;
    /// two threads racing here both open the device and one handle wins,
    /// the other closing on drop -- harmless. Reads go through `&File`, which
    /// the kernel serialises, so no lock is held across them.
    fn urandom() -> Result<&'static File, Error> {
        if let Some(file) = URANDOM.get() {
            return Ok(file);
        }
        let file =
            File::open("/dev/urandom").map_err(|e| Error(format!("open /dev/urandom: {e}")))?;
        Ok(URANDOM.get_or_init(|| file))
    }

    fn fill_urandom(buf: &mut [u8]) -> Result<(), Error> {
        let mut file = urandom()?;
        fill_with(buf, |chunk| file.read(chunk))
            .map_err(|e| Error(format!("read /dev/urandom: {}", e.0)))
    }

    /// `getrandom(2)` with flags 0: blocks until the kernel pool is seeded, which
    /// `/dev/urandom` does not. `ENOSYS` (kernel before 3.17) falls back to
    /// `/dev/urandom`.
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
        use rusty_libc::{rand::getrandom, Errno};
        let mut unsupported = false;
        let result = fill_with(buf, |chunk| match getrandom(chunk, 0) {
            Ok(n) => Ok(n),
            Err(Errno::ENOSYS) => {
                unsupported = true;
                Ok(chunk.len()) // stop the loop; the buffer is refilled below
            }
            Err(e) => Err(std::io::Error::from_raw_os_error(e.0)),
        });
        if unsupported {
            return fill_urandom(buf);
        }
        result.map_err(|e| Error(format!("getrandom: {}", e.0)))
    }

    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
        fill_urandom(buf)
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
        // `cb_buffer` is a ULONG: fill in ULONG-sized chunks rather than
        // truncating `buf.len()` and reporting the unfilled tail as random.
        for chunk in buf.chunks_mut(ULONG::MAX as usize) {
            // SAFETY: `BCryptGenRandom` with a null algorithm handle and
            // `BCRYPT_USE_SYSTEM_PREFERRED_RNG` writes exactly `cb_buffer`
            // bytes into `pb_buffer`, which is the live, mutable `chunk`,
            // whose length fits ULONG by construction.
            let status = unsafe {
                BCryptGenRandom(
                    std::ptr::null_mut(),
                    chunk.as_mut_ptr(),
                    chunk.len() as ULONG,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            };
            if status != 0 {
                return Err(Error(format!(
                    "BCryptGenRandom failed with NTSTATUS 0x{status:08x}"
                )));
            }
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

    use std::io::{Error as IoError, ErrorKind};

    /// A scripted source: each call pops the next step.
    fn scripted(
        steps: Vec<Result<usize, ErrorKind>>,
    ) -> impl FnMut(&mut [u8]) -> std::io::Result<usize> {
        let mut steps = steps.into_iter();
        move |chunk: &mut [u8]| match steps.next().expect("source called more than scripted") {
            Ok(n) => {
                let len = n.min(chunk.len());
                chunk[..len].fill(0xAB);
                Ok(n)
            }
            Err(kind) => Err(IoError::from(kind)),
        }
    }

    #[test]
    fn short_and_interrupted_reads_are_continued_until_full() {
        let mut buf = [0u8; 10];
        let src = scripted(vec![Ok(3), Err(ErrorKind::Interrupted), Ok(4), Ok(3)]);
        fill_with(&mut buf, src).expect("fill");
        assert_eq!(buf, [0xAB; 10]);
    }

    #[test]
    fn a_read_that_makes_no_progress_is_an_error() {
        let mut buf = [0u8; 4];
        let err = fill_with(&mut buf, scripted(vec![Ok(2), Ok(0)])).unwrap_err();
        assert!(err.to_string().contains("no bytes"), "{err}");
    }

    #[test]
    fn a_source_claiming_more_than_it_was_given_is_an_error() {
        let mut buf = [0u8; 4];
        assert!(fill_with(&mut buf, scripted(vec![Ok(5)])).is_err());
    }

    #[test]
    fn a_failing_source_propagates_the_error() {
        let mut buf = [0u8; 4];
        let err = fill_with(
            &mut buf,
            scripted(vec![Ok(1), Err(ErrorKind::PermissionDenied)]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("permission denied"), "{err}");
    }

    #[test]
    fn large_fills_are_complete() {
        // Larger than one getrandom(2) call may be guaranteed to return.
        let out = bytes(1 << 20).expect("1 MiB");
        assert_eq!(out.len(), 1 << 20);
        assert!(out.iter().any(|&b| b != 0));
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
