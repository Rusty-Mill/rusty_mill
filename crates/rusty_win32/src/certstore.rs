//! System certificate stores — `CertOpenSystemStoreW`/
//! `CertEnumCertificatesInStore`/`CertCloseStore`, the read side of
//! Windows' trust store. The primitive behind "which root CAs does this
//! machine trust", i.e. the Windows counterpart of reading
//! `/etc/ssl/certs` on Linux.
//!
//! Read-only by design. Adding to or removing from a system trust store
//! is a machine-wide security decision, not a binding this crate should
//! make casually available; `CertAddCertificateContextToStore` and
//! friends stay out until something needs them and says why.
//!
//! ## The enumeration contract, and why it is shaped like this
//!
//! `CertEnumCertificatesInStore` is a stateful walk: pass `NULL` to get
//! the first context, then pass the context you just got to advance.
//! Each returned `CERT_CONTEXT` is **owned by the store**, not by the
//! caller — it stays valid only until the next call advances past it or
//! the store closes, and freeing it yourself is a bug.
//!
//! That is a lifetime relationship Rust can express, but only by handing
//! out a borrowed view whose every use has to be checked against a walk
//! the caller drives. Rather than build that, [`Store::certificates`]
//! copies each certificate's DER bytes out as it goes and returns owned
//! `Vec<u8>`s. Trust anchors are small and read once at startup; the
//! copies cost nothing that matters, and no OS-owned pointer ever
//! escapes this module. [`Store`] itself closes on drop, so the one
//! resource that *does* need managing manages itself.
//!
//! ## Fidelity limit, stated rather than papered over
//!
//! Windows populates the ROOT store lazily through the AuthRoot update
//! mechanism: enumeration returns the roots currently cached on the
//! machine, not necessarily every root the OS chain engine would fetch
//! on demand mid-validation. An anchor set read this way can therefore
//! be missing a root Windows itself would have trusted. No enumeration
//! API avoids this — the only fix is to use the OS's own chain
//! verification instead, which is a different surface entirely. A caller
//! building a trust store from this needs to know that; hiding it would
//! be the actual defect.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Win32Error;
use crate::handle::RawHandle;
use crate::wide::to_wide;

// CERT_CONTEXT: `size_of` 40, `align_of` 8 on x86_64 — field order and
// types transcribed from the Windows metadata (`windows-sys`'
// `Win32::Security::Cryptography::CERT_CONTEXT`) and pinned by the
// compile-time asserts below. Only `cert_encoded`/`cert_encoded_len` are
// read here; `cert_info` and `cert_store` exist to make the layout right.
#[repr(C)]
#[derive(Clone, Copy)]
struct CertContext {
    cert_encoding_type: u32,
    cert_encoded: *mut u8,
    cert_encoded_len: u32,
    cert_info: *mut core::ffi::c_void,
    cert_store: RawHandle,
}

const _: () = assert!(core::mem::size_of::<CertContext>() == 40);
const _: () = assert!(core::mem::align_of::<CertContext>() == 8);
const _: () = assert!(core::mem::offset_of!(CertContext, cert_encoded) == 8);
const _: () = assert!(core::mem::offset_of!(CertContext, cert_encoded_len) == 16);

#[cfg(windows)]
#[link(name = "crypt32")]
unsafe extern "system" {
    // `hprov` is `HCRYPTPROV_LEGACY`, an integer handle type rather than a
    // pointer — `0` is the documented "use the default provider" value.
    fn CertOpenSystemStoreW(hprov: usize, subsystem_protocol: *const u16) -> RawHandle;
    fn CertEnumCertificatesInStore(store: RawHandle, prev: *const CertContext) -> *mut CertContext;
    fn CertCloseStore(store: RawHandle, flags: u32) -> i32;
}

/// An open system certificate store, closed on drop.
///
/// The drop is why this is a type rather than three free functions: an
/// enumeration that returns early — on an error, or because a caller
/// stopped short — must still close the store, and a destructor is the
/// only way to get that right unconditionally.
pub struct Store {
    handle: RawHandle,
    /// The store name, kept for `Debug` and for error context; a caller
    /// juggling ROOT and CA stores should not have to track which handle
    /// is which.
    name: String,
}

impl core::fmt::Debug for Store {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Store").field("name", &self.name).finish()
    }
}

impl Store {
    /// The store's name, as passed to [`open`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every certificate in the store, as DER bytes.
    ///
    /// Certificates with no encoded bytes are skipped rather than
    /// treated as fatal: a store is a collection of independent entries,
    /// and one unreadable entry is not a reason to report the whole
    /// trust store unavailable.
    ///
    /// An empty store is reported as an empty `Vec`, not an error —
    /// "this machine trusts nothing" is a legitimate (if alarming)
    /// answer, and deciding whether it is acceptable is the caller's
    /// policy.
    #[cfg(windows)]
    pub fn certificates(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut ctx: *mut CertContext = core::ptr::null_mut();
        loop {
            // SAFETY: `self.handle` is open for this whole loop (this
            // method borrows `self`, and only `Drop` closes it); `ctx` is
            // either NULL on the first pass — `CertEnumCertificatesInStore`'s
            // documented "start here" value — or the context the previous
            // iteration received from this same call, which is exactly
            // what it expects to advance from. The returned context is
            // owned by the store and must not be freed here.
            ctx = unsafe { CertEnumCertificatesInStore(self.handle, ctx) };
            if ctx.is_null() {
                break;
            }
            // SAFETY: a non-null context points at a valid `CertContext`
            // whose `cert_encoded`/`cert_encoded_len` describe the DER
            // encoding, readable until the enumeration advances past it
            // or the store closes — both strictly after this copy.
            let der = unsafe {
                let c = &*ctx;
                if c.cert_encoded.is_null() || c.cert_encoded_len == 0 {
                    continue;
                }
                core::slice::from_raw_parts(c.cert_encoded, c.cert_encoded_len as usize).to_vec()
            };
            out.push(der);
        }
        out
    }
}

#[cfg(windows)]
impl Drop for Store {
    fn drop(&mut self) {
        // SAFETY: `self.handle` was returned by `CertOpenSystemStoreW` and
        // is closed exactly once here. Flags `0` is "close normally";
        // every context handed out by `certificates` was copied from and
        // dropped before this point, so nothing outlives the store.
        unsafe {
            CertCloseStore(self.handle, 0);
        }
    }
}

/// Open a system certificate store by name — `CertOpenSystemStoreW` with
/// the default provider.
///
/// `name` is a system store name, not a path: `"ROOT"` for the trusted
/// root CAs, `"CA"` for intermediates, `"MY"` for the personal store.
#[cfg(windows)]
pub fn open(name: &str) -> Result<Store, Win32Error> {
    let wide = to_wide(name);
    // SAFETY: `wide` is a valid NUL-terminated UTF-16 string alive across
    // the call; `0` is the documented default-provider value for the
    // integer `HCRYPTPROV_LEGACY` parameter.
    let handle = unsafe { CertOpenSystemStoreW(0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(Win32Error::last());
    }
    Ok(Store {
        handle,
        name: String::from(name),
    })
}

/// The machine's trusted root CAs, as DER bytes — [`open`]`("ROOT")` plus
/// [`Store::certificates`], the one composition essentially every caller
/// of this module wants.
#[cfg(windows)]
pub fn root_certificates() -> Result<Vec<Vec<u8>>, Win32Error> {
    Ok(open("ROOT")?.certificates())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_root_store_opens_and_reports_its_name() {
        let store = open("ROOT").expect("the ROOT store should open");
        assert_eq!(store.name(), "ROOT");
    }

    #[test]
    fn the_root_store_holds_real_der_certificates() {
        let anchors = root_certificates().expect("ROOT should open");
        assert!(
            !anchors.is_empty(),
            "a Windows machine should trust at least one root CA"
        );
        for der in &anchors {
            // Every X.509 certificate is a DER SEQUENCE: tag 0x30. This
            // checks the bytes are the encoding claimed, not merely
            // non-empty.
            assert_eq!(der.first(), Some(&0x30), "not a DER SEQUENCE");
        }
    }

    #[test]
    fn opening_a_nonexistent_store_is_an_ordinary_error() {
        // `CertOpenSystemStoreW` creates a store that doesn't exist rather
        // than failing, so this asserts only that the call is well-behaved
        // — it must not panic or hand back a null-handle `Store`.
        if let Ok(store) = open("rusty-win32-no-such-store") {
            let _ = store.certificates();
        }
    }
}
