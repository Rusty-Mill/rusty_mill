//! `BCryptGenRandom` — the CNG cryptographically-secure random source,
//! Windows' counterpart to Linux `getrandom(2)` (and to
//! `rusty_libc::rand`'s own syscall wrapper). One function, because one
//! function is what a CSPRNG consumer needs: fill a buffer with bytes
//! nothing can predict.
//!
//! Deliberately *not* in [`crate::security`], despite the family
//! resemblance: that module is the ACL/SID surface — who may do what to
//! which object — reached through `advapi32.dll`. This is `bcrypt.dll`
//! and a different number space (see [`NtStatus`] below). Merging them
//! would mean one module with two error conventions.
//!
//! ## Why this returns [`NtStatus`] and not `Win32Error`
//!
//! `BCryptGenRandom` reports failure as an `NTSTATUS`, not a
//! `GetLastError` code — a genuinely different numbering, in which `0` is
//! success and the high bit marks failure. Every other fallible wrapper
//! in this crate returns `Win32Error`, and `conpty` even converts its
//! `HRESULT`s into one rather than leaking a second convention. That
//! conversion works there because `HRESULT_FROM_WIN32` embeds the
//! original Win32 code in its low 16 bits, so it can be *recovered*.
//! `NTSTATUS` has no such embedding: the only faithful translation is
//! `RtlNtStatusToDosError`, an `ntdll` export this crate does not bind
//! and would not admit for one call.
//!
//! So the raw status comes back typed instead. A caller that wants a
//! Win32 code can run its own `RtlNtStatusToDosError`; a caller that
//! wants to classify the status directly (rustils' `platform-windows`
//! does exactly this, mapping to a portable `ErrorKind`) keeps the
//! information it needs. Silently widening it into `Win32Error` would
//! have put an NTSTATUS bit pattern into a field documented to hold a
//! Win32 error code — the kind of quiet lie this crate's error handling
//! exists to avoid.

#[cfg(windows)]
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut core::ffi::c_void,
        buffer: *mut u8,
        buffer_len: u32,
        flags: u32,
    ) -> i32;
}

/// An `NTSTATUS` — the kernel/CNG status code space, distinct from
/// `Win32Error`'s `GetLastError` space. Success is exactly `0`
/// (`STATUS_SUCCESS`); a negative value is a failure, since `NTSTATUS`
/// encodes severity in its top two bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NtStatus(pub i32);

impl NtStatus {
    /// `STATUS_SUCCESS`.
    pub const SUCCESS: NtStatus = NtStatus(0);

    /// The raw numeric status.
    pub const fn code(self) -> i32 {
        self.0
    }

    /// Whether this status encodes a failure (severity bits set — i.e.
    /// the value is negative when read as a signed 32-bit integer).
    pub const fn is_err(self) -> bool {
        self.0 < 0
    }
}

impl core::fmt::Display for NtStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NTSTATUS 0x{:08X}", self.0 as u32)
    }
}

impl core::error::Error for NtStatus {}

/// `BCRYPT_USE_SYSTEM_PREFERRED_RNG` — draw from the system-preferred RNG
/// without opening an algorithm-provider handle first. Documented as
/// valid only when the algorithm handle is `NULL`, which is why
/// [`random_bytes`] passes the two together and exposes neither.
pub const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

/// Fill `buf` with cryptographically-secure random bytes —
/// `BCryptGenRandom(NULL, buf, buf.len(), BCRYPT_USE_SYSTEM_PREFERRED_RNG)`.
///
/// Safe, not `unsafe`: unlike most of this crate's wrappers there is no
/// caller-supplied handle whose validity has to be promised. The only
/// pointer involved is derived from `buf` itself, so the borrow checker
/// already guarantees everything the call needs.
///
/// A buffer longer than `u32::MAX` is filled only up to that length,
/// which is reported back — the same short-fill contract a `read`-shaped
/// call has, rather than silently truncating the length argument by
/// wrapping. In practice no CSPRNG consumer asks for 4 GiB of entropy in
/// one call; the bound is stated so a caller that somehow does gets a
/// number it can loop on instead of a wrong answer.
#[cfg(windows)]
pub fn random_bytes(buf: &mut [u8]) -> Result<usize, NtStatus> {
    let len = buf.len().min(u32::MAX as usize);
    if len == 0 {
        return Ok(0);
    }
    // SAFETY: `buf[..len]` is a valid writable region of exactly `len`
    // bytes for the duration of the call (`len <= buf.len()`), and `len`
    // fits `u32` by construction; a NULL algorithm handle is required by
    // — and only valid with — `BCRYPT_USE_SYSTEM_PREFERRED_RNG`.
    let status = unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            buf.as_mut_ptr(),
            len as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    let status = NtStatus(status);
    if status.is_err() {
        Err(status)
    } else {
        Ok(len)
    }
}

/// Off-Windows stand-in so the module's *types* stay reachable for
/// documentation and cross-platform compilation of callers' error
/// plumbing. Never returns a value — there is no CNG here.
#[cfg(not(windows))]
pub fn random_bytes(_buf: &mut [u8]) -> Result<usize, NtStatus> {
    Err(NtStatus(i32::MIN))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn fills_the_whole_buffer_and_reports_its_length() {
        let mut buf = [0u8; 64];
        let n = random_bytes(&mut buf).expect("BCryptGenRandom should succeed");
        assert_eq!(n, 64);
        // A 64-byte draw being all-zero has probability 2^-512; treating
        // it as a failure is safe and catches a wrapper that never wrote.
        assert!(buf.iter().any(|&b| b != 0), "buffer left untouched");
    }

    #[test]
    fn two_draws_differ() {
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        random_bytes(&mut a).expect("first draw");
        random_bytes(&mut b).expect("second draw");
        assert_ne!(a, b, "two independent draws should not collide");
    }

    #[test]
    fn an_empty_buffer_is_a_clean_no_op() {
        assert_eq!(random_bytes(&mut []).expect("empty draw"), 0);
    }
}
