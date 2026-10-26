//! CSPRNG (RFC v2 R5+, D15, first Security surface slice): `BCryptGenRandom`
//! with the system preferred RNG, Windows' equivalent of Linux's
//! `getrandom(2)` (see `platform::security`'s module doc comment).
//!
//! Also the user identity of a process's token ([`UserSid`]) and the
//! owner-only DACL ([`restrict_to_owner`]) that together stand in for
//! Unix's `SO_PEERCRED` uid check and `0600` socket file. Neither has a
//! `rusty_win32` binding, so both stay on windows-sys under `track-w`.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::path::Path;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

// The CSPRNG, credential and trust-anchor families route through
// `rusty_win32` under `track-w`; the identity and DACL calls below do
// not, so the windows-sys surface stays referenced in both arms.
use crate::ffi::win32_surface as w;
use crate::sys::errmap::{self, nt_err};
use crate::sys::handle::OwnedWinHandle;
use crate::util::wide::to_wide_nul;

/// Fill `buf` with `BCryptGenRandom(NULL, buf, buf.len(), BCRYPT_USE_SYSTEM_PREFERRED_RNG)`.
/// A null algorithm handle plus this flag is CNG's documented way to draw
/// from the system RNG without opening an algorithm provider handle first.
///
/// Track W (D-15): `rusty_win32::crypto::random_bytes`. The donor is a
/// *safe* function — the only pointer involved is derived from `buf`, so
/// there was never a validity promise for a caller to make — and it
/// returns its `NTSTATUS` typed rather than widening it into a
/// `Win32Error`, which is exactly what lets this arm feed the identical
/// [`nt_err`] classification the windows-sys arm uses. Bit-identical
/// `PlatformError` either way.
///
/// It also reports how many bytes it filled, clamping at `u32::MAX`
/// where the windows-sys arm below truncates the length with an
/// unchecked `as u32`. A short fill is therefore visible here and would
/// be silent there; for a CSPRNG draw (never remotely 4 GiB) the two
/// agree in every reachable case, and the loop makes the track-w arm
/// correct rather than merely equal.
#[cfg(feature = "track-w")]
pub fn fill_random(buf: &mut [u8]) -> Result<()> {
    let mut filled = 0usize;
    while filled < buf.len() {
        let n = rusty_win32::crypto::random_bytes(&mut buf[filled..])
            .map_err(|e| nt_err(e.code(), "BCryptGenRandom", OsStr::new("")))?;
        if n == 0 {
            // Unreachable for a non-empty slice (the donor only returns
            // 0 for an empty one), but looping forever on a surprise is
            // worse than reporting it.
            return Err(PlatformError::new(
                ErrorKind::Other,
                OsCode::None,
                "BCryptGenRandom made no progress",
            ));
        }
        filled += n;
    }
    Ok(())
}

#[cfg(not(feature = "track-w"))]
pub fn fill_random(buf: &mut [u8]) -> Result<()> {
    // SAFETY: `buf` is a valid, writable region of the stated length for
    // the duration of the call; a null algorithm handle is required by
    // (and only valid with) `BCRYPT_USE_SYSTEM_PREFERRED_RNG`.
    let status = unsafe {
        w::BCryptGenRandom(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            w::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(nt_err(status, "BCryptGenRandom", OsStr::new("")));
    }
    Ok(())
}

/// `CredentialStore` (RFC v2 R5+, D15, Phase 6 item 2, rustils#76): Credential
/// Manager's `CRED_TYPE_GENERIC` credential type is the generic secret slot
/// every non-Windows-logon consumer uses — `TargetName` maps to
/// `CredentialStore`'s `service`, `UserName` to `account`,
/// `CredentialBlob` to the raw secret bytes.
/// Write `secret` under `(target_name, user_name)`, persisted past the
/// current logon session (`CRED_PERSIST_LOCAL_MACHINE`). `CredWriteW`
/// replaces any existing credential with the same `TargetName`/`Type` in
/// place — `CredentialStore::set`'s "replace" contract for free, no
/// separate delete-then-write needed.
///
/// Track W (D-15): `rusty_win32::credential::write`. The donor's wrapper
/// takes `&str` where this takes `&OsStr` — the one lossy edge in this
/// slice, and a real one: a target or account name that is not valid
/// UTF-8 has no `&str` to become. Rather than silently mangling it
/// through `to_string_lossy` (which would write the secret under a
/// *different* key than the caller asked for, and then fail to find it
/// on read), a non-UTF-8 name is refused with `InvalidInput`. Windows
/// credential names are UTF-16 in the OS and ASCII in every real
/// consumer, so this is a boundary that should never be hit; it is
/// checked rather than assumed because the failure mode of assuming is
/// a silently misfiled secret.
#[cfg(feature = "track-w")]
pub fn credential_set(target_name: &OsStr, user_name: &OsStr, secret: &[u8]) -> Result<()> {
    let target = utf8_name(target_name, "CredWriteW")?;
    let user = utf8_name(user_name, "CredWriteW")?;
    rusty_win32::credential::write(target, user, secret)
        .map_err(|e| errmap::trackw_err("CredWriteW", e))
}

/// A credential name as `&str`, or `InvalidInput` — see
/// [`credential_set`]'s own comment for why this refuses rather than
/// lossily converts.
#[cfg(feature = "track-w")]
fn utf8_name<'a>(name: &'a OsStr, op: &'static str) -> Result<&'a str> {
    name.to_str().ok_or_else(|| {
        PlatformError::new(ErrorKind::InvalidInput, OsCode::None, op).with_path(name)
    })
}

#[cfg(not(feature = "track-w"))]
pub fn credential_set(target_name: &OsStr, user_name: &OsStr, secret: &[u8]) -> Result<()> {
    let mut target_w = to_wide_nul(target_name);
    let mut user_w = to_wide_nul(user_name);
    let mut blob = secret.to_vec();

    // SAFETY: plain zero-initialization of a `#[repr(C)]` struct whose
    // all-zero bit pattern is valid (every field is either an integer or
    // a nullable pointer) — every field this call actually uses is set
    // explicitly below before the one place it's read.
    let mut cred: w::CREDENTIALW = unsafe { std::mem::zeroed() };
    cred.Type = w::CRED_TYPE_GENERIC;
    cred.TargetName = target_w.as_mut_ptr();
    cred.CredentialBlobSize = blob.len() as u32;
    cred.CredentialBlob = blob.as_mut_ptr();
    cred.Persist = w::CRED_PERSIST_LOCAL_MACHINE;
    cred.UserName = user_w.as_mut_ptr();

    // SAFETY: every pointer field in `cred` (`TargetName`, `UserName`,
    // `CredentialBlob`) points into a still-live local (`target_w`/
    // `user_w`/`blob`) that outlives this call; `CredWriteW` only reads
    // through them for the duration of the call and retains none of
    // them afterward (it copies what it needs into its own storage).
    let ok = unsafe { w::CredWriteW(&cred, 0) };
    if ok == 0 {
        return Err(errmap::last_win32_err("CredWriteW", OsStr::new("")));
    }
    Ok(())
}

/// Read the `CRED_TYPE_GENERIC` credential stored under `target_name`, or
/// `Ok(None)` if Credential Manager has nothing under that name
/// (`ERROR_NOT_FOUND` — `CredentialStore::get`'s clean-miss contract, not
/// an error).
///
/// Track W (D-15): `rusty_win32::credential::read`, which already folds
/// `ERROR_NOT_FOUND` into `Ok(None)` — the same clean-miss contract this
/// function documents, arrived at independently on both sides — and
/// copies the secret out before `CredFree`ing, so the raw `CREDENTIALW`
/// never reaches this crate at all. Same `&str` name constraint as
/// [`credential_set`].
#[cfg(feature = "track-w")]
pub fn credential_get(target_name: &OsStr) -> Result<Option<Vec<u8>>> {
    let target = utf8_name(target_name, "CredReadW")?;
    rusty_win32::credential::read(target).map_err(|e| errmap::trackw_err("CredReadW", e))
}

#[cfg(not(feature = "track-w"))]
pub fn credential_get(target_name: &OsStr) -> Result<Option<Vec<u8>>> {
    let target_w = to_wide_nul(target_name);
    let mut pcred: *mut w::CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target_w` is a valid NUL-terminated wide string outliving
    // the call; `pcred` is a valid out-pointer `CredReadW` writes a
    // freshly allocated `CREDENTIALW*` into on success, freed exactly
    // once below via `CredFree`.
    let ok = unsafe { w::CredReadW(target_w.as_ptr(), w::CRED_TYPE_GENERIC, 0, &mut pcred) };
    if ok == 0 {
        let e = errmap::last_win32_err("CredReadW", OsStr::new(""));
        return if e.kind == ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(e)
        };
    }
    // SAFETY: `ok != 0` guarantees `CredReadW` populated `pcred` with a
    // valid, non-null allocation whose `CredentialBlob`/
    // `CredentialBlobSize` describe a valid region for at least as long
    // as `pcred` itself is valid — this dereference happens strictly
    // before the `CredFree` call below releases that allocation.
    let cred = unsafe { &*pcred };
    // `CREDENTIALW::CredentialBlob` is documented as possibly `NULL`
    // when `CredentialBlobSize` is 0 — exactly the shape produced by
    // storing an empty secret via `credential_set`. `from_raw_parts`
    // requires a non-null pointer even for a zero-length slice, so that
    // case is handled here before ever calling it, instead of being
    // passed straight through as UB.
    let secret = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
        Vec::new()
    } else {
        // SAFETY: guarded above — `cred.CredentialBlob` is non-null and
        // `cred.CredentialBlobSize` is nonzero, so `CredReadW`'s
        // guarantee that they describe a valid, initialized region
        // applies; this copy happens strictly before the `CredFree`
        // call below releases that allocation.
        unsafe { std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize) }
            .to_vec()
    };
    // SAFETY: `pcred` is the exact allocation `CredReadW` returned above,
    // freed exactly once, only after the copy above is done reading
    // through it.
    unsafe {
        w::CredFree(pcred as *const std::ffi::c_void);
    }
    Ok(Some(secret))
}

// --- TrustAnchors (rustils#88) ----------------------------------------

/// Enumerate the system ROOT certificate store, returning each anchor's
/// DER bytes.
///
/// `CertOpenSystemStoreW(NULL, "ROOT")` + `CertEnumCertificatesInStore`
/// in a loop + `CertCloseStore`. Each enumeration step returns a
/// `CERT_CONTEXT` borrowed from the store (not an allocation the caller
/// frees — the store owns it, and passing it back in as the `prev`
/// argument is what advances the iteration), so this copies the DER out
/// rather than holding pointers past the loop.
///
/// **Fidelity limit, documented rather than worked around** (see
/// `platform::security::TrustAnchors`): Windows populates the ROOT store
/// lazily via AuthRoot, so enumeration returns the roots currently
/// cached on the machine — not necessarily every root the OS chain
/// engine would have fetched on demand mid-validation. An anchor set
/// read this way can therefore be missing a root the OS itself would
/// have trusted. There is no enumeration API that avoids this; the only
/// fix is using the OS's own chain verification instead, which is a
/// different surface this workspace deliberately does not offer (Linux
/// has no counterpart to it).
///
/// Track W (D-15): `rusty_win32::certstore::root_certificates`, which
/// runs the identical open/enumerate/close walk with the identical
/// per-entry tolerance (an entry with no encoded bytes is skipped, not
/// fatal) and closes the store via `Drop` rather than a trailing call —
/// so an early return cannot leak it, which the windows-sys arm below
/// achieves only because it has no early return to worry about. The
/// empty-store-is-`NotFound` rule stays here: the donor reports an empty
/// store as an empty `Vec`, correctly treating "trusts nothing" as a
/// legitimate answer and leaving the policy to its caller. This *is*
/// that caller.
#[cfg(feature = "track-w")]
pub fn load_anchors() -> Result<Vec<Vec<u8>>> {
    let anchors = rusty_win32::certstore::root_certificates().map_err(|e| {
        errmap::trackw_err("CertOpenSystemStoreW(ROOT)", e).with_path(OsStr::new("ROOT"))
    })?;
    if anchors.is_empty() {
        return Err(PlatformError::new(
            ErrorKind::NotFound,
            OsCode::None,
            "ROOT certificate store held no usable certificates",
        ));
    }
    Ok(anchors)
}

#[cfg(not(feature = "track-w"))]
pub fn load_anchors() -> Result<Vec<Vec<u8>>> {
    let store_name = to_wide_nul(OsStr::new("ROOT"));
    // SAFETY: `hprov` is `HCRYPTPROV_LEGACY`, an integer handle type
    // (not a pointer) — `0` is the documented "use the default provider"
    // value for opening a named system store. `store_name` is a valid
    // NUL-terminated wide string alive across the call.
    let store = unsafe { w::CertOpenSystemStoreW(0, store_name.as_ptr()) };
    if store.is_null() {
        return Err(errmap::last_win32_err(
            "CertOpenSystemStoreW(ROOT)",
            OsStr::new("ROOT"),
        ));
    }

    let mut anchors: Vec<Vec<u8>> = Vec::new();
    let mut ctx: *mut w::CERT_CONTEXT = std::ptr::null_mut();
    loop {
        // SAFETY: `store` is open for the whole loop; `ctx` is either
        // null (start of enumeration) or the context the previous
        // iteration returned, which is exactly what this call expects to
        // advance from. The returned context is owned by the store.
        ctx = unsafe { w::CertEnumCertificatesInStore(store, ctx) };
        if ctx.is_null() {
            break;
        }
        // SAFETY: a non-null context from the enumerator points at a
        // valid `CERT_CONTEXT` whose `pbCertEncoded`/`cbCertEncoded`
        // describe the DER encoding, valid until the enumeration
        // advances or the store closes — both strictly after this copy.
        let der = unsafe {
            let c = &*ctx;
            if c.pbCertEncoded.is_null() || c.cbCertEncoded == 0 {
                // Per-anchor tolerance, same contract as every backend:
                // an entry we can't read is skipped, not fatal.
                continue;
            }
            std::slice::from_raw_parts(c.pbCertEncoded, c.cbCertEncoded as usize).to_vec()
        };
        anchors.push(der);
    }

    // SAFETY: `store` was opened above and is closed exactly once here;
    // the enumeration has finished, so no borrowed context outlives it,
    // and every DER was copied out rather than referenced. Flags `0`
    // means "close normally, don't check for outstanding references".
    unsafe {
        w::CertCloseStore(store, 0);
    }

    if anchors.is_empty() {
        return Err(PlatformError::new(
            ErrorKind::NotFound,
            OsCode::None,
            "ROOT certificate store held no usable certificates",
        ));
    }
    Ok(anchors)
}

/// The user SID from a process token's `TokenUser`, in its own buffer.
///
/// The buffer is `u64`s so the `TOKEN_USER` header at its start (which
/// holds a pointer) is aligned; the SID it points to lives later in the
/// same buffer, so the pointer stays valid as long as `self` does.
pub struct UserSid {
    buf: Vec<u64>,
}

impl UserSid {
    /// The SID of the user this process runs as.
    pub fn current() -> Result<Self> {
        // SAFETY: `GetCurrentProcess` takes no arguments and returns a
        // pseudo handle that is always valid in this process and needs
        // no close.
        let process = unsafe { w::GetCurrentProcess() };
        // SAFETY: the pseudo handle above is valid for the whole call.
        unsafe { Self::of_process(process) }
    }

    /// The SID of the user process `pid` runs as. Fails if the process
    /// does not exist or this process may not query it.
    pub fn of_pid(pid: u32) -> Result<Self> {
        // SAFETY: `pid` may name no process; `OpenProcess` reports that
        // as a null return, checked below.
        let raw = unsafe { w::OpenProcess(w::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        // SAFETY: `raw` is a failure sentinel or a fresh handle that
        // nothing else owns.
        let process = unsafe { OwnedWinHandle::from_raw(raw) }
            .ok_or_else(|| errmap::last_win32_err("OpenProcess", OsStr::new("")))?;
        // SAFETY: `process` is open for the whole call.
        unsafe { Self::of_process(process.as_raw()) }
    }

    /// # Safety
    /// `process` must be a valid process handle (or the current-process
    /// pseudo handle) with `PROCESS_QUERY_LIMITED_INFORMATION` access.
    unsafe fn of_process(process: w::HANDLE) -> Result<Self> {
        let mut raw: w::HANDLE = std::ptr::null_mut();
        // SAFETY: `process` is valid per this function's contract; `raw`
        // is a valid out-pointer.
        if unsafe { w::OpenProcessToken(process, w::TOKEN_QUERY, &mut raw) } == 0 {
            return Err(errmap::last_win32_err("OpenProcessToken", OsStr::new("")));
        }
        // SAFETY: the call succeeded, so `raw` is a fresh token handle
        // that nothing else owns.
        let token = unsafe { OwnedWinHandle::from_raw(raw) }
            .ok_or_else(|| errmap::last_win32_err("OpenProcessToken", OsStr::new("")))?;

        let mut len: u32 = 0;
        // SAFETY: a null buffer of length 0 asks only for the size,
        // written to `len`; the expected failure is checked below.
        let ok = unsafe {
            w::GetTokenInformation(
                token.as_raw(),
                w::TokenUser,
                std::ptr::null_mut(),
                0,
                &mut len,
            )
        };
        // SAFETY: `GetLastError` has no preconditions.
        if ok != 0 || unsafe { w::GetLastError() } != w::ERROR_INSUFFICIENT_BUFFER {
            return Err(errmap::last_win32_err(
                "GetTokenInformation",
                OsStr::new(""),
            ));
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buf` is writable for at least `len` bytes and outlives
        // the call; `len` is a valid out-pointer.
        let ok = unsafe {
            w::GetTokenInformation(
                token.as_raw(),
                w::TokenUser,
                buf.as_mut_ptr().cast(),
                len,
                &mut len,
            )
        };
        if ok == 0 {
            return Err(errmap::last_win32_err(
                "GetTokenInformation",
                OsStr::new(""),
            ));
        }
        Ok(Self { buf })
    }

    fn as_psid(&self) -> w::PSID {
        // SAFETY: `buf` was filled by a successful `TokenUser` query, so
        // it starts with an aligned, initialized `TOKEN_USER`.
        unsafe { (*self.buf.as_ptr().cast::<w::TOKEN_USER>()).User.Sid }
    }

    /// The SID in string form (`S-1-5-21-...`).
    pub fn to_sid_string(&self) -> Result<String> {
        let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: `as_psid` points at a valid SID inside `self.buf`;
        // `raw` is a valid out-pointer for the `LocalAlloc`ed string.
        if unsafe { w::ConvertSidToStringSidW(self.as_psid(), &mut raw) } == 0 {
            return Err(errmap::last_win32_err(
                "ConvertSidToStringSidW",
                OsStr::new(""),
            ));
        }
        // SAFETY: on success `raw` is a NUL-terminated UTF-16 string;
        // the length scan stops at that NUL, and the slice is copied
        // before the string is freed.
        let text = unsafe {
            let mut n = 0;
            while *raw.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(raw, n))
        };
        // SAFETY: `raw` was allocated by `ConvertSidToStringSidW`, whose
        // contract is a single `LocalFree`.
        unsafe {
            w::LocalFree(raw.cast());
        }
        Ok(text)
    }
}

impl PartialEq for UserSid {
    fn eq(&self, other: &Self) -> bool {
        // SAFETY: both pointers name valid SIDs inside their buffers,
        // which outlive the call.
        unsafe { w::EqualSid(self.as_psid(), other.as_psid()) != 0 }
    }
}

/// A security descriptor allocated by
/// `ConvertStringSecurityDescriptorToSecurityDescriptorW`, freed on drop.
struct LocalSecurityDescriptor(w::PSECURITY_DESCRIPTOR);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a successful conversion, whose
        // contract is a single `LocalFree`.
        unsafe {
            w::LocalFree(self.0);
        }
    }
}

/// Replace `path`'s DACL with one protected entry granting full access
/// to the current user's SID, so no other account (an administrator
/// included, short of taking ownership) can open it.
///
/// The SID is the user's, not the `OW` owner-rights alias, because an
/// elevated administrator's new files are owned by the Administrators
/// group. The DACL is applied through a handle opened with
/// `FILE_FLAG_OPEN_REPARSE_POINT`: an `AF_UNIX` socket file is a reparse
/// point, and path-based calls that follow it fail.
pub fn restrict_to_owner(path: &Path) -> Result<()> {
    let sid = UserSid::current()?.to_sid_string()?;
    let sddl = to_wide_nul(OsStr::new(&format!("D:P(A;;FA;;;{sid})")));
    let mut raw: w::PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated and outlives the call; `raw` is a
    // valid out-pointer; the size out-pointer is optional.
    let ok = unsafe {
        w::ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            w::SDDL_REVISION_1,
            &mut raw,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || raw.is_null() {
        return Err(errmap::last_win32_err(
            "ConvertStringSecurityDescriptorToSecurityDescriptorW",
            OsStr::new(""),
        ));
    }
    let descriptor = LocalSecurityDescriptor(raw);

    let wide = to_wide_nul(path.as_os_str());
    // SAFETY: `wide` is NUL-terminated and outlives the call; the
    // security attributes and template are optional and null.
    let raw = unsafe {
        w::CreateFileW(
            wide.as_ptr(),
            w::WRITE_DAC,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE | w::FILE_SHARE_DELETE,
            std::ptr::null(),
            w::OPEN_EXISTING,
            w::FILE_FLAG_OPEN_REPARSE_POINT | w::FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: `raw` is a failure sentinel or a fresh handle that nothing
    // else owns.
    let file = unsafe { OwnedWinHandle::from_raw(raw) }
        .ok_or_else(|| errmap::last_win32_err("CreateFileW", path.as_os_str()))?;
    // SAFETY: `file` is open with `WRITE_DAC`; `descriptor` holds a valid
    // self-relative descriptor that outlives the call.
    if unsafe {
        w::SetKernelObjectSecurity(file.as_raw(), w::DACL_SECURITY_INFORMATION, descriptor.0)
    } == 0
    {
        return Err(errmap::last_win32_err(
            "SetKernelObjectSecurity",
            path.as_os_str(),
        ));
    }
    Ok(())
}
