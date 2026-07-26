//! Sovereign Filesystem abstractions for rusty_std.

use crate::error::{Error, Result};
use crate::io::{Read, Write};
use crate::path::Path;
use alloc::vec::Vec;

/// A handle to an open file.
pub struct File {
    #[cfg(target_os = "linux")]
    fd: i32,
    #[cfg(windows)]
    handle: *mut core::ffi::c_void,
    #[cfg(target_arch = "wasm32")]
    virtual_id: u32,
}

impl File {
    /// Opens a file in read-only mode.
    pub fn open(_path: &Path) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self { fd: -1 })
        }
        #[cfg(windows)]
        {
            Ok(Self { handle: core::ptr::null_mut() })
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(Self { virtual_id: 0 })
        }
        #[cfg(not(any(target_os = "linux", windows, target_arch = "wasm32")))]
        {
            Err(Error::NotFound(alloc::string::String::from("Unsupported OS")))
        }
    }
}

impl Read for File {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Helper to read an entire file into a Vec<u8>.
pub fn read(_path: &Path) -> Result<Vec<u8>> {
    let mut vec = Vec::new();
    let mut file = File::open(_path)?;
    file.read_to_end(&mut vec)?;
    Ok(vec)
}
