//! Sovereign Filesystem abstractions for rusty_std.

use crate::error::{Error, Result};
use crate::io::{Read, Write};
use crate::path::Path;
use alloc::vec::Vec;

#[cfg(target_os = "linux")]
use alloc::ffi::CString;

/// A handle to an open file.
pub struct File {
    #[cfg(target_os = "linux")]
    fd: i32,
    #[cfg(windows)]
    handle: rusty_win32::RawHandle,
    #[cfg(target_arch = "wasm32")]
    virtual_id: u32,
}

#[cfg(target_os = "linux")]
fn map_errno(op: &str, err: rusty_libc::Errno) -> Error {
    Error::Io(err.code(), alloc::format!("{op}: {err}"))
}

#[cfg(windows)]
fn map_win32(op: &str, err: rusty_win32::Win32Error) -> Error {
    Error::Io(err.code() as i32, alloc::format!("{op}: {err}"))
}

#[cfg(target_os = "linux")]
fn path_to_cstring(path: &Path) -> Result<CString> {
    CString::new(path.as_str()).map_err(|_| {
        Error::InvalidArgument(alloc::string::String::from(
            "path contains an interior NUL byte",
        ))
    })
}

impl File {
    /// Opens an existing file in read-only mode.
    pub fn open(path: &Path) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let c_path = path_to_cstring(path)?;
            let fd = rusty_libc::fd::open(&c_path, rusty_libc::fd::O_RDONLY, 0)
                .map_err(|e| map_errno("open", e))?;
            Ok(Self { fd })
        }
        #[cfg(windows)]
        {
            let handle = rusty_win32::fs::open_file(path.as_str(), rusty_win32::fs::GENERIC_READ)
                .map_err(|e| map_win32("open", e))?;
            Ok(Self { handle })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Ok(Self { virtual_id: 0 })
        }
        #[cfg(not(any(target_os = "linux", windows, target_arch = "wasm32")))]
        {
            let _ = path;
            Err(Error::NotFound(alloc::string::String::from(
                "Unsupported OS",
            )))
        }
    }

    /// Creates a file for writing, truncating it if it already exists (and
    /// creating it if it doesn't).
    pub fn create(path: &Path) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let c_path = path_to_cstring(path)?;
            let flags =
                rusty_libc::fd::O_WRONLY | rusty_libc::fd::O_CREAT | rusty_libc::fd::O_TRUNC;
            let fd =
                rusty_libc::fd::open(&c_path, flags, 0o644).map_err(|e| map_errno("create", e))?;
            Ok(Self { fd })
        }
        #[cfg(windows)]
        {
            let handle =
                rusty_win32::fs::create_file(path.as_str(), rusty_win32::fs::GENERIC_WRITE)
                    .map_err(|e| map_win32("create", e))?;
            Ok(Self { handle })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Ok(Self { virtual_id: 0 })
        }
        #[cfg(not(any(target_os = "linux", windows, target_arch = "wasm32")))]
        {
            let _ = path;
            Err(Error::NotFound(alloc::string::String::from(
                "Unsupported OS",
            )))
        }
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        #[cfg(target_os = "linux")]
        {
            rusty_libc::fd::read(self.fd, buf).map_err(|e| map_errno("read", e))
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.handle` was opened by `File::open`/`File::create`
            // and is only ever closed once, in `Drop`, after which this
            // `File` (and any `&mut` borrow of it) can no longer be used.
            unsafe { rusty_win32::fs::read_file(self.handle, buf) }
                .map_err(|e| map_win32("read", e))
        }
        #[cfg(any(target_arch = "wasm32", not(any(target_os = "linux", windows))))]
        {
            let _ = buf;
            Ok(0)
        }
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        #[cfg(target_os = "linux")]
        {
            rusty_libc::fd::write(self.fd, buf).map_err(|e| map_errno("write", e))
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.handle` was opened by `File::open`/`File::create`
            // and is only ever closed once, in `Drop`, after which this
            // `File` (and any `&mut` borrow of it) can no longer be used.
            unsafe { rusty_win32::fs::write_file(self.handle, buf) }
                .map_err(|e| map_win32("write", e))
        }
        #[cfg(any(target_arch = "wasm32", not(any(target_os = "linux", windows))))]
        {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let _ = rusty_libc::fd::close(self.fd);
        }
        #[cfg(windows)]
        {
            // SAFETY: `self.handle` is a currently-open handle uniquely
            // owned by this `File`, closed exactly once here and never
            // used again afterward.
            let _ = unsafe { rusty_win32::close(self.handle) };
        }
    }
}

/// Helper to read an entire file into a `Vec<u8>`.
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let mut vec = Vec::new();
    let mut file = File::open(path)?;
    file.read_to_end(&mut vec)?;
    Ok(vec)
}

/// Helper to write `contents` to `path`, creating or truncating it first.
pub fn write(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real syscall-backed round-trip: proves `File`/`read`/`write` actually
    // hit the OS (rusty_libc on Linux, rusty_win32 on Windows) rather than
    // being the no-op stubs this module started as.
    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(alloc::format!(
            "rusty_std_fs_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn write_then_read_round_trips_real_file_contents() {
        let tmp = temp_path("roundtrip");
        let path_str = tmp.to_str().expect("temp path should be valid UTF-8");
        let path = crate::path::Path::new(path_str);

        write(path, b"hello, sovereign filesystem").expect("write should succeed");
        let contents = read(path).expect("read should succeed");
        assert_eq!(contents, b"hello, sovereign filesystem");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn create_truncates_an_existing_file() {
        let tmp = temp_path("truncate");
        let path_str = tmp.to_str().expect("temp path should be valid UTF-8");
        let path = crate::path::Path::new(path_str);

        write(path, b"this is the original, longer content").expect("first write should succeed");
        write(path, b"short").expect("second write (create) should truncate");
        let contents = read(path).expect("read should succeed");
        assert_eq!(contents, b"short");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn open_on_a_nonexistent_path_fails() {
        let tmp = temp_path("does-not-exist");
        let path_str = tmp.to_str().expect("temp path should be valid UTF-8");
        let path = crate::path::Path::new(path_str);

        match File::open(path) {
            Ok(_) => panic!("opening a nonexistent file should fail, not succeed"),
            Err(err) => assert!(matches!(err, Error::Io(_, _))),
        }
    }
}
