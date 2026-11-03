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

/// An account as the system database holds it: what a process needs to
/// run as it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: std::path::PathBuf,
    pub shell: std::path::PathBuf,
}

/// The account named `name`, or `None` if there is none (or the lookup
/// otherwise fails, on the same terms as [`user_name`]).
pub fn account(name: &str) -> Option<Account> {
    let name = std::ffi::CString::new(name).ok()?;
    with_growing_buffer(|buf| {
        // SAFETY: see `user_name`: `getpwnam_r` has the same contract, and
        // `pw_dir`/`pw_shell` point into `buf` like `pw_name`, copied out
        // before `buf` is dropped. `name` is a valid C string for the call.
        unsafe {
            let mut pwd: c::passwd = std::mem::zeroed();
            let mut result: *mut c::passwd = std::ptr::null_mut();
            let rc = c::getpwnam_r(
                name.as_ptr(),
                &mut pwd,
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
            let text = |p: *const std::ffi::c_char| {
                if p.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            };
            let account = Account {
                name: text(pwd.pw_name),
                uid: pwd.pw_uid,
                gid: pwd.pw_gid,
                home: text(pwd.pw_dir).into(),
                shell: text(pwd.pw_shell).into(),
            };
            (0, Some(account))
        }
    })
}

/// Every group `name` belongs to, `gid` (its primary group) included, as
/// `initgroups` would set them; `None` if the lookup fails.
pub fn groups(name: &str, gid: u32) -> Option<Vec<u32>> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut cap: usize = 64;
    loop {
        let mut list = vec![0u32; cap];
        let mut count = i32::try_from(cap).ok()?;
        // SAFETY: `name` is a valid C string; `list` holds `count` gids,
        // and `getgrouplist` writes at most that many, setting `count` to
        // how many it found (or needs, when it returns -1).
        let rc = unsafe { c::getgrouplist(name.as_ptr(), gid, list.as_mut_ptr(), &mut count) };
        let found = usize::try_from(count).ok()?;
        if rc >= 0 {
            list.truncate(found);
            return Some(list);
        }
        // Too small: `count` says how many there are. Grow, within reason.
        cap = found.max(cap.checked_mul(2)?);
        if cap > 1 << 16 {
            return None;
        }
    }
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

    #[test]
    fn root_is_an_account_in_its_own_group() {
        let root = account("root").expect("root");
        assert_eq!((root.uid, root.gid), (0, 0));
        assert!(root.home.is_absolute(), "{root:?}");
        assert!(groups("root", 0).expect("groups").contains(&0));
    }

    #[test]
    fn an_unknown_name_is_no_account() {
        assert_eq!(account("no-such-user-rusty-mux"), None);
        assert_eq!(account("bad\0name"), None);
    }
}
