//! Sovereign Path and PathBuf definitions for rusty_std.

use alloc::string::String;
use core::fmt;

/// An owned, mutable path string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathBuf {
    inner: String,
}

impl PathBuf {
    /// Creates an empty PathBuf.
    pub fn new() -> Self {
        Self {
            inner: String::new(),
        }
    }

    /// Borrows this PathBuf as a Path reference.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.inner)
    }

    /// Pushes a path segment onto PathBuf.
    pub fn push(&mut self, path: &str) {
        if !self.inner.is_empty() && !self.inner.ends_with('/') && !self.inner.ends_with('\\') {
            self.inner.push('/');
        }
        self.inner.push_str(path);
    }
}

impl Default for PathBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for PathBuf {
    fn from(s: &str) -> Self {
        Self {
            inner: String::from(s),
        }
    }
}

/// A borrowed path slice.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Path {
    inner: str,
}

impl Path {
    /// Directly wraps a string slice into a Path.
    pub fn new(s: &str) -> &Self {
        unsafe { &*(s as *const str as *const Path) }
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.inner
    }
}

impl fmt::Display for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}
