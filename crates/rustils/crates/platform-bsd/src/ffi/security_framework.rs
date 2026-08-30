//! The exact Security.framework / CoreFoundation items this backend is
//! permitted to touch (rustils#88, TrustAnchors slice) — macOS only.
//!
//! Same curation discipline as [`super::libc_surface`] (RFC v2 §6):
//! anything not declared here is out of bounds for `sys/`, and widening
//! this list is a reviewed decision. Unlike that module these are
//! hand-written `extern` declarations rather than re-exports, because
//! `libc` does not cover Apple's frameworks and this workspace takes no
//! third-party binding crate for them.
//!
//! Gated to `macos` alone, not the whole BSD cfg: Security.framework is
//! Darwin's, and the other four targets in this crate's gate keep their
//! trust anchors in files (see `sys::trust_anchors`).
//!
//! ## Core Foundation ownership, which the SAFETY comments in `sys/`
//! depend on
//!
//! CF has two naming conventions that decide who releases what:
//!
//! - **Create/Copy** — the caller owns the returned reference and must
//!   `CFRelease` it exactly once. That is `SecTrustCopyAnchorCertificates`
//!   and `SecCertificateCopyData` below.
//! - **Get** — the reference is borrowed from its container and must
//!   *not* be released. That is `CFArrayGetValueAtIndex`,
//!   `CFDataGetBytePtr`, and `CFDataGetLength`.
//!
//! Releasing a Get result is a use-after-free; failing to release a
//! Copy result is a leak. Every `unsafe` block in
//! `sys::trust_anchors`'s Darwin path cites which rule it is following.

#![cfg(target_os = "macos")]

use std::ffi::c_void;

/// Opaque CF/Security reference types. All are pointers to opaque
/// structs; this crate never dereferences one directly, only passes them
/// back to the functions below.
pub type CFTypeRef = *const c_void;
/// A `CFArrayRef` holding `SecCertificateRef`s.
pub type CFArrayRef = *const c_void;
/// A `CFDataRef` holding a certificate's DER encoding.
pub type CFDataRef = *const c_void;
/// A `SecCertificateRef`, borrowed from the anchors array.
pub type SecCertificateRef = *const c_void;

/// CF's signed index type — `long`, so pointer-width.
pub type CFIndex = isize;
/// Security.framework's result code. `0` is `errSecSuccess`.
pub type OSStatus = i32;

/// `errSecSuccess` — the only non-error `OSStatus` this module cares
/// about; every other value is surfaced as a `PlatformError` carrying
/// the raw code.
pub const ERR_SEC_SUCCESS: OSStatus = 0;

#[link(name = "Security", kind = "framework")]
extern "C" {
    /// **Copy rule** — on success writes an owned `CFArrayRef` of
    /// `SecCertificateRef` into `anchors`, which the caller must
    /// `CFRelease`.
    ///
    /// Returns the system's *built-in* anchor certificates. This is the
    /// fidelity limit `platform::security::TrustAnchors` documents for
    /// macOS: it is not the same thing as the user's effective trust,
    /// which additionally depends on per-domain trust settings this call
    /// does not consult.
    pub fn SecTrustCopyAnchorCertificates(anchors: *mut CFArrayRef) -> OSStatus;

    /// **Copy rule** — returns an owned `CFDataRef` holding `cert`'s DER
    /// encoding, or null on failure. Caller must `CFRelease` it.
    pub fn SecCertificateCopyData(cert: SecCertificateRef) -> CFDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    /// Number of elements in a `CFArrayRef`.
    pub fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;

    /// **Get rule** — borrowed element, must not be released.
    pub fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;

    /// Byte length of a `CFDataRef`'s contents.
    pub fn CFDataGetLength(data: CFDataRef) -> CFIndex;

    /// **Get rule** — borrowed pointer to a `CFDataRef`'s bytes, valid
    /// only while that `CFDataRef` is alive.
    pub fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;

    /// Release one reference. Must be called exactly once for every
    /// Create/Copy result above.
    pub fn CFRelease(cf: CFTypeRef);
}
