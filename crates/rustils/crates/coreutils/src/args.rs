//! Panic-free argument collection shared by every CLI binary in this
//! crate.
//!
//! Round-6 monorepo review: 15 of the 21 `src/bin/*.rs` binaries called
//! `std::env::args()`, which panics if any argv entry is not valid
//! Unicode. On Windows, a non-Unicode command-line argument (e.g. a
//! mis-encoded filename handed off from another tool, which surfaces as
//! an unpaired UTF-16 surrogate) crashed the tool instead of erroring
//! cleanly or working correctly. `std::env::args_os()` never panics —
//! it hands back `OsString`s — but every one of these tools already
//! treats its argv as text (flag matching against string literals,
//! `usize`/`f64` parsing, and paths handed to `rpath`, which requires
//! `&str`). `collect_lossy` is the one place that requirement is made
//! explicit: it replaces invalid UTF-8/UTF-16 sequences with U+FFFD
//! instead of the implicit panic `std::env::args()` performs internally.
use std::ffi::OsString;

/// Collect an iterator of (possibly non-Unicode) `OsString` arguments
/// into `String`s, lossily replacing any invalid UTF-8/UTF-16 sequence
/// with U+FFFD rather than panicking. Callers pass `std::env::args_os()`
/// in `main`; tests pass hand-built `OsString`s to exercise the
/// non-Unicode path without depending on real process argv.
pub fn collect_lossy<I: IntoIterator<Item = OsString>>(args: I) -> Vec<String> {
    args.into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_lossy_passes_valid_unicode_through_unchanged() {
        let args = vec![
            OsString::from("rcp"),
            OsString::from("-r"),
            OsString::from("café"),
        ];
        assert_eq!(
            collect_lossy(args),
            vec!["rcp".to_string(), "-r".to_string(), "café".to_string()]
        );
    }

    // The regression this guards: `std::env::args()` panics with
    // "invalid utf-8 sequence" the instant one argv entry isn't valid
    // Unicode. `collect_lossy` is built on `to_string_lossy`, which by
    // contract never panics, so this same input must survive.
    #[test]
    #[cfg(unix)]
    fn collect_lossy_does_not_panic_on_invalid_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        // 0xFF is not a valid UTF-8 lead or continuation byte anywhere.
        let bad = OsString::from_vec(vec![b'f', b'o', 0xFF, b'o']);
        let result = collect_lossy(vec![bad]);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].contains('\u{FFFD}'),
            "expected a lossy replacement character, got {:?}",
            result[0]
        );
    }

    #[test]
    #[cfg(windows)]
    fn collect_lossy_does_not_panic_on_unpaired_surrogate() {
        use std::os::windows::ffi::OsStringExt;

        // An unpaired high surrogate: a legal argv code unit on
        // Windows (argv is UTF-16), but not representable as UTF-8 —
        // exactly what `std::env::args()` panics on.
        let bad = OsString::from_wide(&[0x0066, 0x0066, 0xD800, 0x0066]);
        let result = collect_lossy(vec![bad]);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].contains('\u{FFFD}'),
            "expected a lossy replacement character, got {:?}",
            result[0]
        );
    }
}
