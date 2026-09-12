//! NUL-terminated UTF-16 encoding, the string representation every `*W`
//! (wide) Win32 API in this crate expects. Shared here because every FFI
//! module that calls a `*W` function needed this exact conversion and had
//! independently hand-rolled it.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::Win32Error;

/// Encodes `s` as NUL-terminated UTF-16 for a `*W` Win32 call. Rejects any
/// embedded `'\0'` with [`Win32Error::ERROR_INVALID_PARAMETER`] rather than
/// silently truncating at it — a Win32 `*W` API reads a single
/// NUL-terminated string, so an embedded NUL would otherwise cut the
/// string short with no signal to the caller. Matches `CString::new`'s own
/// rejection of embedded NULs, and this crate's Linux sibling
/// (`rusty_std::fs::path_to_cstring`, built on `CString::new`).
pub(crate) fn to_wide(s: &str) -> Result<Vec<u16>, Win32Error> {
    if s.contains('\0') {
        return Err(Win32Error::ERROR_INVALID_PARAMETER);
    }
    Ok(s.encode_utf16().chain(core::iter::once(0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_wide_encodes_an_ordinary_string_nul_terminated() {
        let wide = to_wide("hi").expect("a plain string should encode fine");
        assert_eq!(wide, alloc::vec![b'h' as u16, b'i' as u16, 0]);
    }

    #[test]
    fn to_wide_rejects_an_embedded_nul_instead_of_truncating() {
        // Before this fix, `to_wide` silently stopped at the embedded NUL
        // (`s.encode_utf16().chain(once(0)).collect()` faithfully encodes
        // every `char` including an embedded `'\0'`, but every `*W` Win32
        // call downstream reads only up to that first NUL) — a caller
        // asking for `"secret\0.txt"` would silently operate on `"secret"`
        // instead, the same class of bug `CString::new` guards against.
        let err = to_wide("secret\0.txt").expect_err(
            "a string with an embedded NUL should be rejected, not silently truncated",
        );
        assert_eq!(err, Win32Error::ERROR_INVALID_PARAMETER);
    }
}
