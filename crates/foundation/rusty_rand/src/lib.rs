//! OS-backed cryptographically secure random bytes, with no external
//! dependencies.
//!
//! Extracted from three near-identical copies -- `rusty_oauth::rand`,
//! `rusty_uuid`'s private `rand` module, and `sessionmgr-proc`'s
//! `os_random` -- which had each independently arrived at the same
//! two-backend design:
//!
//! - Linux on x86_64/aarch64: `getrandom(2)` with flags 0 (via `rusty_libc`), which
//!   blocks until the kernel pool is initialised. If the kernel lacks it (`ENOSYS`,
//!   before 3.17) the call falls back to `/dev/random`, never `/dev/urandom`.
//! - Other Linux targets: `/dev/random`, which also waits for pool initialisation (on
//!   kernels before 5.6 it can additionally block on a low entropy estimate).
//! - Other Unix (macOS, the BSDs): `/dev/urandom`, the behaviour before this crate gained
//!   `getrandom`. Whether it can return bytes before the system is seeded depends on the OS and
//!   was not checked per BSD here; the initialised-pool guarantee above is Linux-only. The
//!   handle is opened once and cached, and reads take no lock.
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

    /// Where a target gets its bytes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Backend {
        /// `getrandom(2)`, flags 0: blocks until the kernel pool is initialised.
        GetRandom,
        /// `/dev/random`: on Linux it also blocks until the pool is initialised (and, on
        /// kernels before 5.6, whenever its entropy estimate is low).
        DevRandom,
        /// `/dev/urandom` on other Unix; no initialised-pool guarantee is claimed.
        DevUrandom,
    }

    /// Pure so every combination can be tested on one machine.
    pub(super) const fn backend_for(linux: bool, getrandom_supported: bool) -> Backend {
        match (linux, getrandom_supported) {
            (true, true) => Backend::GetRandom,
            (true, false) => Backend::DevRandom,
            (false, _) => Backend::DevUrandom,
        }
    }

    /// `getrandom(2)` is called through `rusty_libc`, which supports x86_64 and aarch64 Linux.
    pub(super) const fn backend() -> Backend {
        backend_for(
            cfg!(target_os = "linux"),
            cfg!(any(target_arch = "x86_64", target_arch = "aarch64")),
        )
    }

    static URANDOM: OnceLock<File> = OnceLock::new();
    static RANDOM: OnceLock<File> = OnceLock::new();

    /// The cached handle for `path`, opened on first use. `OnceLock` can't run a fallible
    /// initializer, so the open happens outside it; two threads racing here both open the
    /// device and one handle wins, the other closing on drop -- harmless. Reads go through
    /// `&File`, which the kernel serialises, so no lock is held across them.
    fn device(cell: &'static OnceLock<File>, path: &str) -> Result<&'static File, Error> {
        if let Some(file) = cell.get() {
            return Ok(file);
        }
        let file = File::open(path).map_err(|e| Error(format!("open {path}: {e}")))?;
        Ok(cell.get_or_init(|| file))
    }

    fn fill_device(cell: &'static OnceLock<File>, path: &str, buf: &mut [u8]) -> Result<(), Error> {
        let mut file = device(cell, path)?;
        fill_with(buf, |chunk| file.read(chunk)).map_err(|e| Error(format!("read {path}: {}", e.0)))
    }

    pub(super) fn fill_dev_random(buf: &mut [u8]) -> Result<(), Error> {
        fill_device(&RANDOM, "/dev/random", buf)
    }

    pub fn fill(buf: &mut [u8]) -> Result<(), Error> {
        match backend() {
            Backend::GetRandom => fill_getrandom(buf),
            Backend::DevRandom => fill_dev_random(buf),
            Backend::DevUrandom => fill_device(&URANDOM, "/dev/urandom", buf),
        }
    }

    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    fn fill_getrandom(_buf: &mut [u8]) -> Result<(), Error> {
        Err(Error(
            "getrandom(2) is not available on this target".to_string(),
        ))
    }

    /// `getrandom(2)`; if the kernel lacks it (`ENOSYS`, before Linux 3.17) the whole
    /// buffer is refilled from `/dev/random`, never from `/dev/urandom`.
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn fill_getrandom(buf: &mut [u8]) -> Result<(), Error> {
        fill_getrandom_with(
            buf,
            |chunk| rusty_libc::rand::getrandom(chunk, 0).map_err(|e| e.0),
            fill_dev_random,
        )
    }

    /// The `getrandom` loop with the system call and the fallback injected, so the
    /// `ENOSYS` handling is testable. `sys` returns the byte count or the errno.
    #[cfg(any(
        test,
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    pub(super) fn fill_getrandom_with(
        buf: &mut [u8],
        mut sys: impl FnMut(&mut [u8]) -> Result<usize, i32>,
        fallback: impl FnOnce(&mut [u8]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        const ENOSYS: i32 = 38;
        let mut unsupported = false;
        let result = fill_with(buf, |chunk| match sys(chunk) {
            Ok(n) => Ok(n),
            Err(ENOSYS) => {
                unsupported = true;
                Ok(chunk.len()) // ends the loop; the buffer is refilled by the fallback
            }
            Err(errno) => Err(std::io::Error::from_raw_os_error(errno)),
        });
        if unsupported {
            return fallback(buf);
        }
        result.map_err(|e| Error(format!("getrandom: {}", e.0)))
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

    #[cfg(unix)]
    mod backend {
        use super::super::imp::{
            backend, backend_for, fill_dev_random, fill_getrandom_with, Backend,
        };
        use super::super::Error;
        use std::cell::Cell;

        #[test]
        fn selection_covers_every_target_class() {
            assert_eq!(backend_for(true, true), Backend::GetRandom);
            assert_eq!(backend_for(true, false), Backend::DevRandom);
            assert_eq!(backend_for(false, true), Backend::DevUrandom);
            assert_eq!(backend_for(false, false), Backend::DevUrandom);
        }

        #[test]
        fn linux_never_selects_urandom_and_this_build_matches_its_target() {
            if cfg!(target_os = "linux") {
                assert_ne!(backend(), Backend::DevUrandom);
            }
            if cfg!(all(
                target_os = "linux",
                any(target_arch = "x86_64", target_arch = "aarch64")
            )) {
                assert_eq!(backend(), Backend::GetRandom);
            }
        }

        const ENOSYS: i32 = 38;
        const EPERM: i32 = 1;
        const EINTR: i32 = 4;

        fn never(_: &mut [u8]) -> Result<(), Error> {
            panic!("the fallback must not run");
        }

        #[test]
        fn a_working_syscall_never_reaches_the_fallback() {
            let mut buf = [0u8; 600];
            fill_getrandom_with(
                &mut buf,
                |c| {
                    c.fill(0xAB);
                    Ok(c.len().min(256))
                },
                never,
            )
            .unwrap();
            assert_eq!(buf, [0xAB; 600]);
        }

        #[test]
        fn enosys_refills_the_whole_buffer_from_the_fallback_once() {
            for first_calls_ok in [0, 1] {
                let calls = Cell::new(0);
                let mut buf = [0u8; 16];
                let r = fill_getrandom_with(
                    &mut buf,
                    |c| {
                        calls.set(calls.get() + 1);
                        if calls.get() <= first_calls_ok {
                            c.fill(0x11);
                            Ok(4)
                        } else {
                            Err(ENOSYS)
                        }
                    },
                    |b| {
                        b.fill(0x22);
                        Ok(())
                    },
                );
                assert!(r.is_ok());
                assert_eq!(buf, [0x22; 16], "stale partial bytes must be replaced");
            }
        }

        /// The real fallback device works (it is what ENOSYS kernels and other Linux targets use).
        #[test]
        fn the_dev_random_fallback_fills_a_buffer() {
            let mut buf = [0u8; 64];
            fill_dev_random(&mut buf).expect("/dev/random");
            assert_ne!(buf, [0u8; 64]);
        }

        #[test]
        fn a_failing_fallback_is_an_error() {
            let mut buf = [0u8; 8];
            let r = fill_getrandom_with(
                &mut buf,
                |_| Err(ENOSYS),
                |_| Err(Error("no /dev/random".into())),
            );
            assert!(r.unwrap_err().to_string().contains("no /dev/random"));
        }

        #[test]
        fn other_errors_fail_closed_without_the_fallback() {
            let mut buf = [0u8; 8];
            let r = fill_getrandom_with(&mut buf, |_| Err(EPERM), never);
            assert!(r.unwrap_err().to_string().contains("getrandom"));
        }

        #[test]
        fn eintr_is_retried() {
            let n = Cell::new(0);
            let mut buf = [0u8; 4];
            fill_getrandom_with(
                &mut buf,
                |c| {
                    n.set(n.get() + 1);
                    if n.get() < 3 {
                        Err(EINTR)
                    } else {
                        c.fill(7);
                        Ok(c.len())
                    }
                },
                never,
            )
            .unwrap();
            assert_eq!((n.get(), buf), (3, [7; 4]));
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
