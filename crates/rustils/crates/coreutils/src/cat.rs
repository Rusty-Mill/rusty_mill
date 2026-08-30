//! `cat` over the platform traits: byte-faithful, backend-agnostic.
//!
//! Bytes in, bytes out — no UTF-8 decoding anywhere (contrast the v1
//! scaffold's `from_utf8_lossy`, which silently corrupted binary data).

use std::ffi::OsStr;

use platform::error::Result;
use platform::fs::{Dir, OpenOptions};

/// Copy the contents of `rel` (under `dir`) into `out`.
pub fn cat(dir: &dyn Dir, rel: &OsStr, out: &mut dyn std::io::Write) -> Result<()> {
    let mut file = dir.open(rel, &OpenOptions::read())?;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| {
            platform::PlatformError::new(
                platform::ErrorKind::BrokenPipe,
                platform::OsCode::Errno(e.raw_os_error().unwrap_or(0)),
                "write stdout",
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_mock::MockDir;

    #[test]
    fn cat_streams_bytes_faithfully() {
        // Includes invalid UTF-8: must pass through untouched.
        let data: &[u8] = b"hello \xff\xfe world";
        let root = MockDir::root().with_file("f.bin", data);
        let mut out = Vec::new();
        cat(&root, OsStr::new("f.bin"), &mut out).expect("cat");
        assert_eq!(out, data);
    }

    #[test]
    fn cat_missing_file_reports_notfound_with_context() {
        let root = MockDir::root();
        let e = cat(&root, OsStr::new("nope"), &mut Vec::new()).expect_err("must fail");
        assert_eq!(e.kind, platform::ErrorKind::NotFound);
        assert!(e.path.is_some(), "error must carry the path");
    }
}
