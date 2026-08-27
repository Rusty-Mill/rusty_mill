//! NUL-terminated UTF-16 encoding, the string representation every `*W`
//! (wide) Win32 API in this crate expects. Shared here because every FFI
//! module that calls a `*W` function needed this exact conversion and had
//! independently hand-rolled it.

extern crate alloc;
use alloc::vec::Vec;

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}
