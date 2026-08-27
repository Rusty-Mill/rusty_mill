//! Windows Credential Manager — `CredReadW`/`CredWriteW`/`CredFree`, the
//! OS-provided secret store a program uses instead of inventing its own
//! file format for a password. The Windows counterpart of the Secret
//! Service/keyring APIs on Linux desktops.
//!
//! Scoped to `CRED_TYPE_GENERIC`, the generic-secret slot every
//! non-Windows-logon consumer uses: `TargetName` is the lookup key,
//! `UserName` the account within it, and `CredentialBlob` the raw secret
//! bytes. The domain/certificate/logon credential types are out of scope
//! — they carry authentication semantics this crate has no business
//! deciding for a caller.
//!
//! Separate from [`crate::security`] on the same grounds [`crate::crypto`]
//! is: that module answers "who may do what to this object" (ACLs, SIDs);
//! this one stores and retrieves a secret. They share `advapi32.dll` and
//! nothing else.
//!
//! Lifetime note that shapes this API: `CredReadW` hands back a
//! heap-allocated `CREDENTIALW` the caller must release with `CredFree`,
//! and the secret bytes live *inside* that allocation. Exposing the raw
//! pointer would make every caller responsible for a free it cannot
//! forget — so [`read`] copies the blob out and frees before returning,
//! and the raw `CREDENTIALW` never crosses this module's boundary.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::Win32Error;
use crate::wide::to_wide;

/// `CRED_TYPE_GENERIC` — the generic-secret credential type.
pub const CRED_TYPE_GENERIC: u32 = 1;

/// `CRED_PERSIST_LOCAL_MACHINE` — the credential survives logoff and
/// reboot, stored per-machine for this user. The other documented values
/// (`SESSION` = 1, `ENTERPRISE` = 3) are deliberately not exposed: a
/// session-scoped secret is not what a credential *store* is for, and the
/// roaming enterprise variant has domain semantics out of this crate's
/// scope.
pub const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

// CREDENTIALW: `size_of` 80, `align_of` 8 on x86_64 — field order and
// types transcribed from the Windows metadata (`windows-sys`'
// `Win32::Security::Credentials::CREDENTIALW`), which is the same
// authority the header carries, and pinned below by a compile-time
// assert so a mistranscription cannot survive a build. Only the fields
// this module sets or reads are named meaningfully; the rest exist to
// make the layout right, since `CredWriteW` reads the whole struct.
#[repr(C)]
#[derive(Clone, Copy)]
struct CredentialW {
    flags: u32,
    kind: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: FileTime,
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut core::ffi::c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

const _: () = assert!(core::mem::size_of::<CredentialW>() == 80);
const _: () = assert!(core::mem::align_of::<CredentialW>() == 8);
const _: () = assert!(core::mem::offset_of!(CredentialW, credential_blob_size) == 32);
const _: () = assert!(core::mem::offset_of!(CredentialW, credential_blob) == 40);
const _: () = assert!(core::mem::offset_of!(CredentialW, persist) == 48);
const _: () = assert!(core::mem::offset_of!(CredentialW, user_name) == 72);

impl Default for CredentialW {
    fn default() -> Self {
        CredentialW {
            flags: 0,
            kind: 0,
            target_name: core::ptr::null_mut(),
            comment: core::ptr::null_mut(),
            last_written: FileTime::default(),
            credential_blob_size: 0,
            credential_blob: core::ptr::null_mut(),
            persist: 0,
            attribute_count: 0,
            attributes: core::ptr::null_mut(),
            target_alias: core::ptr::null_mut(),
            user_name: core::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(
        target_name: *const u16,
        kind: u32,
        flags: u32,
        credential: *mut *mut CredentialW,
    ) -> i32;
    fn CredFree(buffer: *const core::ffi::c_void);
}

/// Store `secret` under `target_name`/`user_name` as a
/// `CRED_TYPE_GENERIC` credential, persisted per
/// [`CRED_PERSIST_LOCAL_MACHINE`].
///
/// `CredWriteW` *replaces* any existing credential with the same
/// `TargetName` and type in place, so a caller implementing "set" needs
/// no delete-then-write dance — that is the API's own documented
/// behavior, not something this wrapper adds.
///
/// Safe, not `unsafe`: every pointer handed to the OS is derived from an
/// argument this function owns for the duration of the call, so there is
/// no validity promise left for a caller to make.
#[cfg(windows)]
pub fn write(target_name: &str, user_name: &str, secret: &[u8]) -> Result<(), Win32Error> {
    let mut target_w = to_wide(target_name);
    let mut user_w = to_wide(user_name);
    let mut blob = secret.to_vec();

    let cred = CredentialW {
        kind: CRED_TYPE_GENERIC,
        target_name: target_w.as_mut_ptr(),
        credential_blob_size: blob.len().min(u32::MAX as usize) as u32,
        credential_blob: blob.as_mut_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        user_name: user_w.as_mut_ptr(),
        ..CredentialW::default()
    };

    // SAFETY: every pointer field in `cred` points into a local
    // (`target_w`/`user_w`/`blob`) that is still alive at this point and
    // outlives the call; `CredWriteW` reads through them only for the
    // duration of the call, copying what it keeps into its own storage,
    // and retains none of them afterward.
    let ok = unsafe { CredWriteW(&cred, 0) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Read the `CRED_TYPE_GENERIC` secret stored under `target_name`.
///
/// Returns `Ok(None)` when Credential Manager has nothing under that name
/// — `CredReadW` reports that as [`Win32Error::ERROR_NOT_FOUND`], which is
/// an ordinary miss rather than a failure, and folding it into the
/// `Option` saves every caller from re-deriving that distinction.
///
/// The returned bytes are a copy: the `CREDENTIALW` allocation
/// `CredReadW` produces is released with `CredFree` before this returns,
/// so nothing the caller holds points into OS-owned memory.
#[cfg(windows)]
pub fn read(target_name: &str) -> Result<Option<Vec<u8>>, Win32Error> {
    let target_w = to_wide(target_name);
    let mut pcred: *mut CredentialW = core::ptr::null_mut();
    // SAFETY: `target_w` is a valid NUL-terminated UTF-16 string alive
    // across the call; `pcred` is a valid out-pointer that `CredReadW`
    // fills with a freshly allocated `CREDENTIALW*` on success.
    let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pcred) };
    if ok == 0 {
        let err = Win32Error::last();
        return if err == Win32Error::ERROR_NOT_FOUND {
            Ok(None)
        } else {
            Err(err)
        };
    }

    // SAFETY: a nonzero return guarantees `pcred` is a valid, non-null
    // allocation whose `credential_blob`/`credential_blob_size` describe
    // a readable region for as long as that allocation lives — this copy
    // completes strictly before the `CredFree` below releases it. A
    // credential written with an empty secret has a null blob pointer
    // with size 0, which `from_raw_parts` would not accept, so that case
    // is answered without forming a slice at all.
    let secret = unsafe {
        let cred = &*pcred;
        if cred.credential_blob.is_null() || cred.credential_blob_size == 0 {
            Vec::new()
        } else {
            core::slice::from_raw_parts(cred.credential_blob, cred.credential_blob_size as usize)
                .to_vec()
        }
    };

    // SAFETY: `pcred` is exactly the allocation `CredReadW` returned, freed
    // exactly once here, after the copy above has finished reading it and
    // with no surviving reference into it.
    unsafe { CredFree(pcred.cast()) };

    Ok(Some(secret))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    // A target name unlikely to collide with anything real on the runner.
    const TARGET: &str = "rusty_win32/test/credential";

    #[test]
    fn write_then_read_round_trips_a_secret() {
        write(TARGET, "tester", b"hunter2").expect("CredWriteW should succeed");
        let got = read(TARGET).expect("CredReadW should succeed");
        assert_eq!(got.as_deref(), Some(&b"hunter2"[..]));
    }

    #[test]
    fn write_replaces_in_place_without_a_delete() {
        const T: &str = "rusty_win32/test/credential/replace";
        write(T, "tester", b"first").expect("first write");
        write(T, "tester", b"second").expect("second write");
        assert_eq!(read(T).expect("read").as_deref(), Some(&b"second"[..]));
    }

    #[test]
    fn reading_an_absent_target_is_a_clean_miss() {
        let got =
            read("rusty_win32/test/credential/definitely-absent").expect("read should succeed");
        assert_eq!(got, None);
    }

    #[test]
    fn binary_secrets_survive_intact() {
        const T: &str = "rusty_win32/test/credential/binary";
        let secret: Vec<u8> = (0u8..=255).collect();
        write(T, "tester", &secret).expect("write");
        assert_eq!(read(T).expect("read").as_deref(), Some(&secret[..]));
    }
}
