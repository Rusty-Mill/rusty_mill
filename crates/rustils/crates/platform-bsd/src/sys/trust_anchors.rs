//! OS trust-anchor loading (rustils#88, the "B1" slice) — the first
//! `security` surface on this backend, which was net-only until now.
//!
//! Two genuinely different mechanisms behind one function, because the
//! BSDs genuinely differ here:
//!
//! - **macOS** keeps its roots in the keychain, reachable only through
//!   Security.framework. No file to read.
//! - **FreeBSD/OpenBSD/NetBSD/DragonFly** keep theirs in PEM files, the
//!   same shape Linux uses — different paths, identical mechanism.
//!
//! This is the one place in this crate where the Darwin-vs-other-BSD
//! split is a real fork rather than the portable-subset arrangement
//! `lib.rs` describes for `net`: there is no intersection to write here,
//! because one side has no files and the other has no framework.
//!
//! Nothing in this module interprets a certificate — no chain building,
//! no signature checks, no ASN.1. See
//! `platform::security::TrustAnchors`.

#![allow(unsafe_code)]

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

fn no_anchors(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::NotFound, OsCode::None, detail)
}

// --- Darwin: Security.framework ---------------------------------------

/// Load the system's built-in anchor certificates via
/// `SecTrustCopyAnchorCertificates`.
///
/// **Fidelity limit, documented rather than worked around** (see
/// `platform::security::TrustAnchors`): this returns the *built-in*
/// roots. A user's effective trust additionally depends on per-domain
/// trust settings — including explicit distrust records — that this call
/// does not consult and that a flat DER list could not express anyway.
/// Walking those domains is a strictly larger surface, deliberately left
/// as follow-up work rather than smuggled in here (rustils#88).
#[cfg(target_os = "macos")]
pub fn load_anchors() -> Result<Vec<Vec<u8>>> {
    use crate::ffi::security_framework as sf;

    let mut array: sf::CFArrayRef = std::ptr::null();
    // SAFETY: `array` is a valid, exclusively borrowed out-param the call
    // writes an owned CFArrayRef into on success (Copy rule — released
    // below). On failure it is left untouched and we never read it.
    let status = unsafe { sf::SecTrustCopyAnchorCertificates(&mut array) };
    if status != sf::ERR_SEC_SUCCESS || array.is_null() {
        return Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::Errno(status),
            "SecTrustCopyAnchorCertificates",
        ));
    }

    // SAFETY: `array` is the non-null owned CFArrayRef just obtained;
    // reading its count borrows it without transferring ownership.
    let count = unsafe { sf::CFArrayGetCount(array) };

    let mut anchors: Vec<Vec<u8>> = Vec::new();
    for i in 0..count {
        // SAFETY: `i` is in `0..count`, so this is an in-bounds element
        // of a live array. Get rule: the element is borrowed from
        // `array` and must NOT be released here.
        let cert = unsafe { sf::CFArrayGetValueAtIndex(array, i) };
        if cert.is_null() {
            continue;
        }
        // SAFETY: `cert` is a live SecCertificateRef borrowed from
        // `array`. Copy rule: `data` is owned by us and released below.
        let data = unsafe { sf::SecCertificateCopyData(cert) };
        if data.is_null() {
            // Per-anchor tolerance, same contract as every backend: an
            // entry whose DER we can't obtain is skipped, not fatal.
            continue;
        }
        // SAFETY: `data` is a live, owned CFDataRef. Get rule for both
        // calls — the pointer and length borrow from `data`, valid until
        // the `CFRelease` below, which happens strictly after the copy.
        let der = unsafe {
            let len = sf::CFDataGetLength(data);
            let ptr = sf::CFDataGetBytePtr(data);
            if ptr.is_null() || len <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ptr, len as usize).to_vec()
            }
        };
        // SAFETY: `data` came from a Copy-rule call and is released
        // exactly once, after the copy above finished reading it.
        unsafe { sf::CFRelease(data) };
        if !der.is_empty() {
            anchors.push(der);
        }
    }

    // SAFETY: `array` came from a Copy-rule call and is released exactly
    // once, after the loop has finished borrowing from it. Every DER was
    // copied out, so nothing outlives this.
    unsafe { sf::CFRelease(array) };

    if anchors.is_empty() {
        return Err(no_anchors(
            "system anchor certificates held no usable entries",
        ));
    }
    Ok(anchors)
}

// --- FreeBSD/OpenBSD/NetBSD/DragonFly: PEM files ----------------------

/// Bundle files, in probe order. First existing one wins.
#[cfg(not(target_os = "macos"))]
const BUNDLE_PATHS: &[&str] = &[
    "/etc/ssl/cert.pem",                      // OpenBSD, FreeBSD base
    "/usr/local/etc/ssl/cert.pem",            // FreeBSD ports, DragonFly
    "/usr/local/share/certs/ca-root-nss.crt", // FreeBSD ca_root_nss port
    "/etc/openssl/certs/ca-certificates.crt", // NetBSD
    "/etc/ssl/certs/ca-certificates.crt",     // where a port installs one
];

/// Certificate directories, consulted only when no bundle exists.
#[cfg(not(target_os = "macos"))]
const DIR_PATHS: &[&str] = &[
    "/etc/ssl/certs",
    "/usr/local/share/certs",
    "/etc/openssl/certs",
];

/// Load anchors from this system's PEM trust store.
///
/// Same probing policy as `platform-linux`'s copy, with BSD paths:
/// `SSL_CERT_FILE`, else `SSL_CERT_DIR`, else the first existing bundle
/// file, else the first existing certificate directory — first match
/// wins exclusively, never a union.
///
/// The PEM/base64 decoding below is a second copy of
/// `platform-linux::sys::trust_anchors`'s, kept deliberately identical
/// rather than factored out — the same call this workspace's parity
/// suites already make, which is to extract once a *third* backend would
/// otherwise mean a third copy. Windows needs neither (it enumerates a
/// store API), so two is where this stops for now.
#[cfg(not(target_os = "macos"))]
pub fn load_anchors() -> Result<Vec<Vec<u8>>> {
    use std::path::Path;

    if let Some(file) = std::env::var_os("SSL_CERT_FILE") {
        let anchors = read_bundle(Path::new(&file));
        return non_empty(anchors, "SSL_CERT_FILE names no usable certificates");
    }
    if let Some(dir) = std::env::var_os("SSL_CERT_DIR") {
        let anchors = read_dir_anchors(Path::new(&dir));
        return non_empty(anchors, "SSL_CERT_DIR names no usable certificates");
    }
    for candidate in BUNDLE_PATHS {
        let path = Path::new(candidate);
        if path.exists() {
            let anchors = read_bundle(path);
            return non_empty(anchors, "CA bundle held no usable certificates");
        }
    }
    for candidate in DIR_PATHS {
        let path = Path::new(candidate);
        if path.is_dir() {
            let anchors = read_dir_anchors(path);
            return non_empty(anchors, "CA directory held no usable certificates");
        }
    }
    Err(no_anchors(
        "no OS trust store found (no SSL_CERT_FILE/SSL_CERT_DIR, no known bundle or directory)",
    ))
}

#[cfg(not(target_os = "macos"))]
fn non_empty(anchors: Vec<Vec<u8>>, detail: &'static str) -> Result<Vec<Vec<u8>>> {
    if anchors.is_empty() {
        return Err(no_anchors(detail));
    }
    Ok(anchors)
}

#[cfg(not(target_os = "macos"))]
fn b64_decode(input: &[u8]) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | u32::from(sextet(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(not(target_os = "macos"))]
const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
#[cfg(not(target_os = "macos"))]
const END: &str = "-----END CERTIFICATE-----";

#[cfg(not(target_os = "macos"))]
fn pem_to_ders(text: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after_begin = &rest[start + BEGIN.len()..];
        let Some(end) = after_begin.find(END) else {
            break;
        };
        if let Some(der) = b64_decode(&after_begin.as_bytes()[..end]) {
            if !der.is_empty() {
                out.push(der);
            }
        }
        rest = &after_begin[end + END.len()..];
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn read_bundle(path: &std::path::Path) -> Vec<Vec<u8>> {
    match std::fs::read_to_string(path) {
        Ok(text) => pem_to_ders(&text),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_dir_anchors(path: &std::path::Path) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        out.extend(read_bundle(&entry.path()));
    }
    out
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    const DER: &[u8] = &[0x30, 0x82, 0x01, 0x0a];

    #[test]
    fn pem_extracts_and_tolerates_a_bad_block() {
        let doc = format!("{BEGIN}\nnot*base64\n{END}\n{BEGIN}\nMIIBCg==\n{END}\n");
        let ders = pem_to_ders(&doc);
        assert_eq!(ders.len(), 1, "the good block must survive the bad one");
        assert_eq!(ders[0], DER);
    }

    #[test]
    fn b64_matches_the_linux_backends_behavior() {
        assert_eq!(b64_decode(b"TWFu").unwrap(), b"Man");
        assert_eq!(b64_decode(b"TQ==").unwrap(), b"M");
        assert!(b64_decode(b"TW*u").is_none());
    }
}
