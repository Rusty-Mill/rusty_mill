//! UTF-16 boundary conversion — the ONE place WTF-16 policy lives
//! (RFC v2 §5.2). Everything above the sys layer traffics in `OsStr`.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

/// NUL-terminated UTF-16 for Win32 `*W` calls.
///
/// `OsStr` on Windows is WTF-8 and round-trips unpaired surrogates
/// losslessly through `encode_wide` — no lossy conversion happens here.
pub fn to_wide_nul(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Length-counted (no NUL) UTF-16 for NT `UNICODE_STRING` names, with `/`
/// separators normalized to the `\` the NT namespace requires. Relative
/// paths only — the ambient-path entry point (`sys::fileio::open_ambient_dir`)
/// is the one place absolute paths are accepted.
pub fn to_wide_nt_component(s: &OsStr) -> Vec<u16> {
    s.encode_wide()
        .map(|u| {
            if u == u16::from(b'/') {
                u16::from(b'\\')
            } else {
                u
            }
        })
        .collect()
}

/// UTF-16 (WTF-16) back to `OsString`, losslessly.
pub fn from_wide(units: &[u16]) -> OsString {
    OsString::from_wide(units)
}

/// Length-counted (no NUL) UTF-16, byte-for-byte — **no** separator
/// normalization, unlike [`to_wide_nt_component`]. For content that must
/// round-trip exactly rather than resolve as a path component: a
/// symlink's print name (`sys::fileio::symlink`/`read_link`, the
/// `Dir::symlink`/`read_link` contract), which stores and returns
/// `target` verbatim, the same way `readlinkat` never normalizes on
/// Linux either.
pub fn to_wide_raw(s: &OsStr) -> Vec<u16> {
    s.encode_wide().collect()
}
