//! Paths a caller names, checked before any I/O.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a caller's path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// It starts at the filesystem root or names a drive.
    Absolute,
    /// It has a `..` component.
    Parent,
    /// It contains a NUL byte.
    Nul,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathError::Absolute => "absolute paths are refused; name a path inside the session",
            PathError::Parent => "`..` is refused; name a path inside the session",
            PathError::Nul => "a path may not contain NUL",
        })
    }
}

impl std::error::Error for PathError {}

/// A path relative to the session root, with no `..` and no root: it can only
/// name something at or below the root, before symlinks are considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPath(PathBuf);

impl SessionPath {
    /// Check `raw`. An empty path, or `.`, is the root itself.
    ///
    /// # Errors
    /// [`PathError`] for an absolute path, a `..` or a NUL.
    pub fn parse(raw: &str) -> Result<Self, PathError> {
        if raw.contains('\0') {
            return Err(PathError::Nul);
        }
        // Both separators, whatever the host: a Windows-style `\..\` must not
        // slip past a Unix parser as one odd file name.
        if raw.starts_with('/') || raw.starts_with('\\') || has_drive(raw) {
            return Err(PathError::Absolute);
        }
        let mut clean = PathBuf::new();
        for part in raw.split(['/', '\\']) {
            match part {
                "" | "." => {}
                ".." => return Err(PathError::Parent),
                name => {
                    // A name the platform reads as a root or prefix anyway.
                    if Path::new(name)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    {
                        return Err(PathError::Absolute);
                    }
                    clean.push(name);
                }
            }
        }
        Ok(SessionPath(clean))
    }

    /// The root itself.
    pub fn root() -> Self {
        SessionPath(PathBuf::new())
    }

    /// The relative path, empty for the root.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Whether this is the root.
    pub fn is_root(&self) -> bool {
        self.0.as_os_str().is_empty()
    }
}

fn has_drive(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_kept_and_cleaned() {
        assert_eq!(
            SessionPath::parse("src/./lib.rs").map(|p| p.0),
            Ok(PathBuf::from("src/lib.rs"))
        );
        assert_eq!(
            SessionPath::parse("a//b/").map(|p| p.0),
            Ok(PathBuf::from("a/b"))
        );
        assert!(SessionPath::parse("").is_ok_and(|p| p.is_root()));
        assert!(SessionPath::parse(".").is_ok_and(|p| p.is_root()));
        assert_eq!(
            SessionPath::parse("..data").map(|p| p.0),
            Ok(PathBuf::from("..data")),
            "a name that merely starts with dots is a name"
        );
    }

    #[test]
    fn escapes_are_refused_before_io() {
        for (raw, why) in [
            ("/etc/passwd", PathError::Absolute),
            ("/proc/self/environ", PathError::Absolute),
            ("\\windows", PathError::Absolute),
            ("C:\\x", PathError::Absolute),
            ("c:x", PathError::Absolute),
            ("..", PathError::Parent),
            ("../x", PathError::Parent),
            ("a/../../x", PathError::Parent),
            ("a\\..\\..\\x", PathError::Parent),
            ("a/b/..", PathError::Parent),
            ("a\0b", PathError::Nul),
        ] {
            assert_eq!(SessionPath::parse(raw), Err(why), "{raw:?}");
        }
    }
}
