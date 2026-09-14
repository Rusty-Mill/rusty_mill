//! Shared filesystem-path ⇄ LSP `file://` URI conversion.
//!
//! Both [`crate::client`]'s `initialize` handshake (`rootUri`/
//! `workspaceFolders[].uri`) and [`crate::core_plugin`]'s per-document
//! handlers need to turn a local filesystem path into an LSP `file://`
//! URI. Per RFC 3986, a raw `format!("file://{}", path.display())` is
//! wrong for any path containing a space, `#`, `?`, or a non-ASCII byte —
//! `#`/`?` start a fragment/query that a spec-compliant parser strips
//! before the rest of the path ever reaches the server, and a bare space
//! breaks re-parsing entirely. This module is the one place both builders
//! delegate to, backed by `rusty_url`'s WHATWG-URL-standard-conformant
//! percent-encoding — the same correctness property the sibling
//! `rusty_lsp` crate's `Uri::join` already provides for its own callers.

use std::path::{Path, PathBuf};

/// Convert a filesystem path to a correctly percent-encoded `file://` URI.
///
/// Infallible for every path this crate actually builds one for (the
/// forge root, and forge-relative document paths resolved to absolute
/// before reaching an LSP handler) — `rusty_url::Url::from_file_path`'s
/// sole failure mode is a non-absolute path, in which case this falls
/// back to the old naive (unencoded) form so a relative-path caller still
/// gets a usable string instead of a panic.
pub(crate) fn path_to_file_uri(path: &Path) -> String {
    rusty_url::Url::from_file_path(path).map_or_else(
        |()| format!("file://{}", path.display()),
        rusty_url::Url::into_string,
    )
}

/// Decode a `file://` URI back to a filesystem path, undoing any
/// percent-encoding [`path_to_file_uri`] applied.
///
/// Falls back to treating `uri` as a bare path if it isn't a well-formed
/// `file://` URI (e.g. it's already a plain filesystem path), so a caller
/// that hasn't fully migrated to URI-shaped values doesn't lose data.
///
/// No call site in this crate converts a server-returned URI back to a
/// filesystem path today (every proxied LSP response stays raw,
/// untouched JSON — see `core_plugin`'s module doc), but this is the
/// shared decode counterpart future call sites should use rather than
/// re-deriving a naive `strip_prefix("file://")`.
#[allow(dead_code)] // no current call site; see doc comment above.
pub(crate) fn file_uri_to_path(uri: &str) -> PathBuf {
    rusty_url::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .unwrap_or_else(|| PathBuf::from(uri))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    const ABS_DIR: &str = "/tmp";
    #[cfg(windows)]
    const ABS_DIR: &str = r"C:\temp";

    #[test]
    fn path_with_space_round_trips() {
        let path = Path::new(ABS_DIR).join("my file.rs");
        let uri = path_to_file_uri(&path);
        assert!(!uri.contains(' '), "space must be percent-encoded: {uri}");
        assert!(uri.contains("%20"), "expected %20 in {uri}");
        assert_eq!(file_uri_to_path(&uri), path);
    }

    #[test]
    fn path_with_hash_round_trips() {
        // `#` starts a URI fragment per RFC 3986; the pre-fix
        // `format!("file://{}", path.display())` builder produced a URI a
        // spec-compliant parser truncates at the `#`, silently dropping
        // "v2.rs" and everything after it.
        let path = Path::new(ABS_DIR).join("utils#v2.rs");
        let uri = path_to_file_uri(&path);
        assert!(!uri.contains('#'), "# must be percent-encoded: {uri}");
        assert!(uri.contains("%23"), "expected %23 in {uri}");
        assert_eq!(file_uri_to_path(&uri), path);
    }

    #[test]
    fn plain_path_falls_back_on_decode() {
        // Backward-compat: a caller that hasn't migrated to URI-shaped
        // values yet still gets its path back unchanged.
        assert_eq!(file_uri_to_path("/tmp/x.rs"), PathBuf::from("/tmp/x.rs"));
    }
}
