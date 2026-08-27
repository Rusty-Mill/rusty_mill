//! Kernel randomness (`getrandom(2)`).

use crate::arch::nr;
use crate::arch::{from_ret, syscall3, Errno};

/// `getrandom(2)` flag: return `EAGAIN` immediately instead of blocking
/// until the kernel's CSPRNG is seeded (only possible very early at boot)
/// or, without [`GRND_RANDOM`], never otherwise.
pub const GRND_NONBLOCK: i32 = 0x0001;
/// `getrandom(2)` flag: draw from the same pool `/dev/random` does
/// (blocking once its entropy estimate is exhausted) instead of the
/// default, `/dev/urandom`-equivalent CSPRNG output. Rarely what a caller
/// wants — the default is cryptographically secure and does not block
/// after early boot.
pub const GRND_RANDOM: i32 = 0x0002;

/// Fill `buf` with random bytes from the kernel, returning the number
/// actually written (short only if interrupted by a signal, or if
/// [`GRND_NONBLOCK`] is set and the pool is not yet ready — this crate's
/// own no-flags default never returns short).
///
/// This crate has no other source of randomness at all: useful for
/// anything wanting genuinely unpredictable values, most concretely a
/// collision- and symlink-race-resistant temporary file/directory name for
/// heredocs or process substitution (a predictable name is exactly the
/// hazard `mktemp`-style tools exist to avoid).
pub fn getrandom(buf: &mut [u8], flags: i32) -> Result<usize, Errno> {
    // getrandom(buf, buflen, flags).
    // SAFETY: `buf` is a valid, exclusively-borrowed slice of `buf.len()`
    // bytes the kernel writes at most that many of.
    let ret = unsafe {
        syscall3(
            nr::GETRANDOM,
            buf.as_mut_ptr() as usize,
            buf.len(),
            flags as usize,
        )
    };
    from_ret(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getrandom_fills_the_whole_buffer() {
        let mut buf = [0u8; 64];
        let n = getrandom(&mut buf, 0).expect("getrandom");
        assert_eq!(n, buf.len());
    }

    #[test]
    fn getrandom_empty_buffer_returns_zero() {
        assert_eq!(getrandom(&mut [], 0), Ok(0));
    }

    #[test]
    fn getrandom_is_not_all_zero_or_trivially_repeated() {
        // Not a statistical test -- just a sanity check that this is
        // reading real kernel randomness rather than, say, a zeroed or
        // uninitialized buffer slipping through unchanged. A single
        // constant byte value across 64 random bytes has probability
        // 2^-504 by chance.
        let mut buf = [0u8; 64];
        getrandom(&mut buf, 0).expect("getrandom");
        assert!(buf.iter().any(|&b| b != buf[0]));
    }

    #[test]
    fn getrandom_two_calls_differ() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        getrandom(&mut a, 0).expect("getrandom a");
        getrandom(&mut b, 0).expect("getrandom b");
        assert_ne!(a, b);
    }
}
