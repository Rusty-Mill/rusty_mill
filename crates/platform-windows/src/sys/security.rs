//! CSPRNG (RFC v2 R5+, D15, first Security surface slice): `BCryptGenRandom`
//! with the system preferred RNG, Windows' equivalent of Linux's
//! `getrandom(2)` (see `platform::security`'s module doc comment).

#![allow(unsafe_code)]

use std::ffi::OsStr;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

// Fully migrated under `track-w` (D-15): with every family in this
// module routed through `rusty_win32`, the windows-sys surface is
// genuinely unreferenced here rather than merely unused-by-accident.
#[cfg(not(feature = "track-w"))]
use crate::ffi::win32_surface as w;
use crate::sys::errmap::{self, nt_err};
#[cfg(not(feature = "track-w"))]
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
    // as `pcred` itself is valid — this copy happens strictly before the
    // `CredFree` call below releases that allocation.
    let secret = unsafe {
        let cred = &*pcred;
        std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize).to_vec()
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
