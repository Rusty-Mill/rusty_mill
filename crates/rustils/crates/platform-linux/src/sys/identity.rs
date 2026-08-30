//! uid/gid → display-name resolution (`getpwuid_r`/`getgrgid_r`),
//! coreutils gap backlog #65's `ls -l` donor material.
//!
//! Deliberately outside `platform::fs`/`UnixMode`: name resolution is an
//! NSS/directory-service lookup, not filesystem metadata — `UnixMode`
//! answers "what does this entry's mode word say", not "what human-
//! readable string does some external database map a number to". A
//! consumer that wants the number (`UnixMode::uid`/`gid`) already has
//! it; this is the separate, optional prettification `coreutils::ls -l`
//! needs to render output that actually reads like real `ls -l`, not a
//! required part of the capability model.
//!
//! Uses the reentrant `_r` variants, not the classic `getpwuid`/
//! `getgrgid`: those return a pointer into a shared static buffer swapped
//! out from under any other caller in the process on the next call —
//! exactly the non-reentrant-global-state hazard this crate's Track P
//! `LAST_ERRNO` commentary already flags elsewhere for the same reason.

#![allow(unsafe_code)]

use std::ffi::CStr;

use crate::ffi::libc_surface as c;

/// Grows a scratch buffer and retries on `ERANGE` ("your buffer was too
/// small") — `getpwuid_r`/`getgrgid_r`'s well-documented contract for a
/// name too long for a first-guess buffer size, not a real error.
fn with_growing_buffer<T>(mut call: impl FnMut(&mut [i8]) -> (i32, Option<T>)) -> Option<T> {
    let mut cap = 1024usize;
    loop {
        let mut buf = vec![0i8; cap];
        let (rc, found) = call(&mut buf);
        if rc == 0 {
            return found;
        }
        if rc == c::ERANGE {
            cap *= 2;
            if cap > 1 << 20 {
                return None; // not a real name; refuse to grow forever
            }
            continue;
        }
        // Any other errno (e.g. a broken NSS module) is not this
        // lookup's job to diagnose — the caller only wants a display
        // string, with numeric-uid fallback already its own honest
        // answer.
        return None;
    }
}

/// The account name for `uid`, or `None` if there is no such account
/// (or the lookup otherwise fails) — the caller falls back to the
/// numeric uid, the same "real answer or an honest absence" shape
/// `Dir::unix_mode` already uses for Windows.
pub fn user_name(uid: u32) -> Option<String> {
    with_growing_buffer(|buf| {
        // SAFETY: `pwd`/`result` are valid, exclusively-owned out-params
        // for the duration of the call; `buf` is a valid, appropriately
        // sized scratch region the call may write into and reference
        // from `pwd.pw_name` afterward.
        unsafe {
            let mut pwd: c::passwd = std::mem::zeroed();
            let mut result: *mut c::passwd = std::ptr::null_mut();
            let rc = c::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            );
            if rc != 0 {
                return (rc, None);
            }
            if result.is_null() {
                return (0, None); // no such uid — not an error
            }
            // SAFETY: on success, `pw_name` points into `buf`, valid and
            // NUL-terminated for as long as `buf` is alive; copied out
            // as an owned `String` before `buf` is dropped.
            let name = CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned();
            (0, Some(name))
        }
    })
}

/// The group name for `gid`, or `None` on the same terms as
/// [`user_name`].
pub fn group_name(gid: u32) -> Option<String> {
    with_growing_buffer(|buf| {
        // SAFETY: see `user_name` — identical contract, `group`/`gr_name`
        // in place of `passwd`/`pw_name`.
        unsafe {
            let mut grp: c::group = std::mem::zeroed();
            let mut result: *mut c::group = std::ptr::null_mut();
            let rc = c::getgrgid_r(
                gid,
                &mut grp,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            );
            if rc != 0 {
                return (rc, None);
            }
            if result.is_null() {
                return (0, None);
            }
            let name = CStr::from_ptr(grp.gr_name).to_string_lossy().into_owned();
            (0, Some(name))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_resolves_to_a_name() {
        // uid/gid 0 is root on every real Linux system, sandboxed or
        // not — the one identity this test can assert without depending
        // on the specific account database of whatever machine runs it.
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert_eq!(group_name(0).as_deref(), Some("root"));
    }

    #[test]
    fn an_unassigned_uid_resolves_to_nothing() {
        assert_eq!(user_name(u32::MAX - 1), None);
        assert_eq!(group_name(u32::MAX - 1), None);
    }
}
