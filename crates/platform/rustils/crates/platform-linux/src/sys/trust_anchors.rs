//! OS trust-anchor loading (rustils#88, the "B1" slice): find where this
//! Linux system keeps its root certificates, read them, hand back raw
//! DER. Nothing here interprets a certificate — no chain building, no
//! signature checks, no ASN.1 (`platform::security::TrustAnchors`'s doc
//! comment has the full reasoning). PEM decoding is a *container* format
//! transform, not certificate parsing: base64 between two marker lines.
//!
//! No `unsafe` in this module despite living under `sys/`: Linux keeps
//! its anchors in ordinary files, so this is `std::fs` and nothing more.
//! It sits here anyway because `sys/` is where mechanism lives in this
//! crate and `security.rs` is where trait impls live — the layering rule
//! is "all unsafe is in `sys/`", not "everything in `sys/` is unsafe".
//!
//! ## Probing policy (rustils#88, decided; `design-discussion-tls.md` Q3)
//!
//! In strict precedence order, first match wins *exclusively* — never a
//! union, so an anchor can never be loaded twice:
//!
//! 1. `SSL_CERT_FILE` — a bundle file the operator named. Honored even
//!    if unreadable or empty: naming it is an explicit instruction, and
//!    silently falling through to a distro default would defeat the
//!    override.
//! 2. `SSL_CERT_DIR` — a directory the operator named, same reasoning.
//! 3. The first distro bundle file that exists.
//! 4. The first distro certificate *directory* that exists, enumerated.
//!
//! Bundle-before-directory (rather than directory-only, or a union) is
//! what makes one policy cover every distro without special-casing:
//! RHEL-family systems ship only a bundle, Debian ships *both* a bundle
//! and a directory of hashed symlinks pointing at the same certificates.
//! Preferring the bundle means Debian reads one file instead of several
//! hundred symlinks, and never double-loads. The directory arm exists for
//! systems that have only that layout.

use std::path::Path;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

/// Bundle files, in probe order. First existing one wins.
const BUNDLE_PATHS: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt", // Debian, Ubuntu, Arch, Alpine
    "/etc/pki/tls/certs/ca-bundle.crt",   // Fedora, RHEL, CentOS
    "/etc/ssl/ca-bundle.pem",             // openSUSE
    "/etc/pki/tls/cacert.pem",            // older RHEL
    "/etc/ssl/cert.pem",                  // Alpine, and a common fallback
];

/// Certificate directories, in probe order. Only consulted when no
/// bundle above exists.
const DIR_PATHS: &[&str] = &[
    "/etc/ssl/certs",     // Debian family (also holds the bundle above)
    "/etc/pki/tls/certs", // RHEL family
];

fn no_anchors(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::NotFound, OsCode::None, detail)
}

/// Decode standard-alphabet base64, ignoring whitespace, stopping at the
/// first `=` pad. Returns `None` on any character outside the alphabet —
/// the caller treats that as "this PEM block is malformed, skip it".
///
/// Hand-rolled because this crate's dependency set is `libc` and
/// `platform`; pulling a base64 crate in to read a certificate bundle
/// would be a new dependency for thirty lines of arithmetic.
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

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

/// Pull every `CERTIFICATE` block out of a PEM document as DER.
///
/// Per-block tolerance is deliberate and is half of this slice's
/// contract: a block whose base64 is malformed is skipped, not fatal.
/// Real bundles pick up damaged entries over time, and one of them must
/// not cost the caller the other several hundred anchors. Non-certificate
/// PEM blocks (a stray private key, a CRL) are ignored by the same
/// mechanism — they simply never match `BEGIN`.
fn pem_to_ders(text: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after_begin = &rest[start + BEGIN.len()..];
        let Some(end) = after_begin.find(END) else {
            // Unterminated block: nothing valid can follow it either.
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

/// Read one bundle file. A file that can't be read yields no anchors
/// rather than an error — the caller decides whether zero overall is
/// fatal.
fn read_bundle(path: &Path) -> Vec<Vec<u8>> {
    match std::fs::read_to_string(path) {
        Ok(text) => pem_to_ders(&text),
        Err(_) => Vec::new(),
    }
}

/// Enumerate a certificate directory. Debian-style hashed symlinks
/// (`3513523f.0`) and plain `.pem`/`.crt` files both work — every entry
/// is simply tried as PEM and contributes whatever certificates it
/// holds. Unreadable entries and subdirectories contribute nothing.
///
/// Deliberately not recursive: no distro nests its trust store, and
/// walking arbitrary depth from `/etc/ssl/certs` is a much bigger promise
/// than this trait makes.
fn read_dir_anchors(path: &Path) -> Vec<Vec<u8>> {
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

/// Load this system's trust anchors as DER, per the probing policy in
/// this module's doc comment.
///
/// Fails closed: zero usable anchors is [`ErrorKind::NotFound`], never
/// `Ok(vec![])`. A caller handed an empty anchor set would trust nothing
/// and fail every TLS connection with a confusing per-connection error;
/// failing here names the real problem instead.
pub fn load_anchors() -> Result<Vec<Vec<u8>>> {
    // 1. `SSL_CERT_FILE`, exclusively.
    if let Some(file) = std::env::var_os("SSL_CERT_FILE") {
        let anchors = read_bundle(Path::new(&file));
        return non_empty(anchors, "SSL_CERT_FILE names no usable certificates");
    }

    // 2. `SSL_CERT_DIR`, exclusively.
    if let Some(dir) = std::env::var_os("SSL_CERT_DIR") {
        let anchors = read_dir_anchors(Path::new(&dir));
        return non_empty(anchors, "SSL_CERT_DIR names no usable certificates");
    }

    // 3. First distro bundle that exists — checked with `exists()` rather
    //    than "first that yields anchors", so a present-but-empty bundle
    //    is a hard error naming the real problem instead of silently
    //    falling through to a directory holding something different.
    for candidate in BUNDLE_PATHS {
        let path = Path::new(candidate);
        if path.exists() {
            let anchors = read_bundle(path);
            return non_empty(anchors, "distro CA bundle held no usable certificates");
        }
    }

    // 4. First distro certificate directory that exists.
    for candidate in DIR_PATHS {
        let path = Path::new(candidate);
        if path.is_dir() {
            let anchors = read_dir_anchors(path);
            return non_empty(anchors, "distro CA directory held no usable certificates");
        }
    }

    Err(no_anchors(
        "no OS trust store found (no SSL_CERT_FILE/SSL_CERT_DIR, no known bundle or directory)",
    ))
}

fn non_empty(anchors: Vec<Vec<u8>>, detail: &'static str) -> Result<Vec<Vec<u8>>> {
    if anchors.is_empty() {
        return Err(no_anchors(detail));
    }
    Ok(anchors)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 4-byte DER-ish payload; this layer never interprets the bytes, so
    // a real certificate would prove nothing extra here.
    const DER: &[u8] = &[0x30, 0x82, 0x01, 0x0a];
    const DER_B64: &str = "MIIBCg==";

    fn pem_block(body: &str) -> String {
        format!("{BEGIN}\n{body}\n{END}\n")
    }

    #[test]
    fn b64_round_trips_the_standard_alphabet() {
        assert_eq!(b64_decode(b"TWFu").unwrap(), b"Man");
        assert_eq!(b64_decode(b"TWE=").unwrap(), b"Ma");
        assert_eq!(b64_decode(b"TQ==").unwrap(), b"M");
        // Whitespace is ignored: PEM wraps at 64 columns.
        assert_eq!(b64_decode(b"TW\nFu ").unwrap(), b"Man");
    }

    #[test]
    fn b64_rejects_characters_outside_the_alphabet() {
        assert!(b64_decode(b"TW*u").is_none());
    }

    #[test]
    fn pem_extracts_every_certificate_block() {
        let doc = format!("{}{}", pem_block(DER_B64), pem_block(DER_B64));
        let ders = pem_to_ders(&doc);
        assert_eq!(ders.len(), 2);
        assert_eq!(ders[0], DER);
    }

    #[test]
    fn pem_skips_a_malformed_block_and_keeps_the_rest() {
        // The contract's per-anchor tolerance: one bad entry in a bundle
        // must not cost the caller the good ones.
        let doc = format!("{}{}", pem_block("not*valid*base64"), pem_block(DER_B64));
        let ders = pem_to_ders(&doc);
        assert_eq!(ders.len(), 1, "the good block must survive the bad one");
        assert_eq!(ders[0], DER);
    }

    #[test]
    fn pem_ignores_surrounding_and_non_certificate_text() {
        let doc = format!(
            "# comment\n-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n{}trailing",
            pem_block(DER_B64)
        );
        assert_eq!(pem_to_ders(&doc), vec![DER.to_vec()]);
    }

    #[test]
    fn pem_stops_at_an_unterminated_block() {
        let doc = format!("{}{BEGIN}\n{DER_B64}\n", pem_block(DER_B64));
        assert_eq!(pem_to_ders(&doc).len(), 1);
    }

    #[test]
    fn empty_document_yields_nothing() {
        assert!(pem_to_ders("").is_empty());
    }
}
