//! Reading files a sandboxed program left behind (review finding 1).
//!
//! The parent reads a solution's output *outside* the sandbox, so the file
//! is an untrusted filesystem object: it may be a symlink to private data,
//! a FIFO that never yields, a device, a hard link to something else, or
//! huge. [`read_untrusted`] opens it exactly once, without following links
//! and without blocking, accepts only a regular file with a single link,
//! and reads at most a fixed number of bytes from that one descriptor. The
//! bytes it returns are the snapshot every later step uses; nothing reopens
//! the path.

use std::io::Read;
use std::path::Path;

use crate::error::RuntimeError;

/// Why an output file was not accepted. Rejection is an outcome (the
/// solution scores the task's floor), not an infrastructure error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// No file at the path.
    Missing,
    /// The path is a symbolic link.
    Symlink,
    /// A FIFO, socket, device or directory rather than a regular file.
    NotRegular,
    /// A regular file with more than one hard link.
    Linked,
    /// Larger than the byte limit.
    TooLarge,
}

impl Rejected {
    /// A short reason for the agent's feedback.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Symlink => "a symbolic link",
            Self::NotRegular => "not a regular file",
            Self::Linked => "a hard link",
            Self::TooLarge => "too large",
        }
    }
}

/// Reads at most `limit` bytes of the regular file at `path`, safely.
///
/// # Errors
/// [`RuntimeError::Io`] only for failures that say nothing about the file
/// itself (for example a read error on an accepted regular file).
pub fn read_untrusted(path: &Path, limit: u64) -> Result<Result<Vec<u8>, Rejected>, RuntimeError> {
    let file = match open_no_follow(path) {
        Ok(file) => file,
        Err(error) => return Ok(Err(classify_open_error(path, &error))),
    };
    let metadata = file
        .metadata()
        .map_err(|e| RuntimeError::io("inspecting an output file", e))?;
    if !metadata.file_type().is_file() {
        return Ok(Err(Rejected::NotRegular));
    }
    if link_count(&metadata) != 1 {
        return Ok(Err(Rejected::Linked));
    }
    read_capped(file, limit)
}

/// Reads at most `limit` bytes from `reader`.
///
/// # Errors
/// [`RuntimeError::Io`] when reading fails.
pub fn read_capped(
    reader: impl Read,
    limit: u64,
) -> Result<Result<Vec<u8>, Rejected>, RuntimeError> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| RuntimeError::io("reading an output file", e))?;
    if bytes.len() as u64 > limit {
        return Ok(Err(Rejected::TooLarge));
    }
    Ok(Ok(bytes))
}

#[cfg(target_os = "linux")]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NOFOLLOW: a symlink fails with ELOOP instead of being followed.
    // O_NONBLOCK: opening a FIFO for reading returns at once instead of
    // waiting for a writer; the file-type check then rejects it.
    // (Both values are per-architecture, hence rusty_libc's constants.)
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rusty_libc::fd::O_NOFOLLOW | rusty_libc::fd::O_NONBLOCK)
        .open(path)
}

#[cfg(not(target_os = "linux"))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::other("symlink"));
    }
    std::fs::File::open(path)
}

fn classify_open_error(path: &Path, error: &std::io::Error) -> Rejected {
    if error.kind() == std::io::ErrorKind::NotFound {
        return Rejected::Missing;
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Rejected::Symlink,
        Ok(_) => Rejected::NotRegular,
        Err(_) => Rejected::Missing,
    }
}

#[cfg(target_os = "linux")]
fn link_count(metadata: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(metadata)
}

#[cfg(not(target_os = "linux"))]
fn link_count(_metadata: &std::fs::Metadata) -> u64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsi-output-{name}-{}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir).expect("clear");
            }
            std::fs::create_dir_all(&dir).expect("create");
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                std::fs::remove_dir_all(&self.0).expect("remove");
            }
        }
    }

    #[test]
    fn reads_a_regular_file_within_the_limit() {
        let dir = Dir::new("regular");
        let path = dir.0.join("out");
        std::fs::write(&path, b"1\n2\n").expect("write");
        assert_eq!(
            read_untrusted(&path, 4).expect("io").map(|b| b.len()),
            Ok(4)
        );
        assert_eq!(
            read_untrusted(&path, 3).expect("io"),
            Err(Rejected::TooLarge)
        );
        assert_eq!(
            read_untrusted(&dir.0.join("none"), 9).expect("io"),
            Err(Rejected::Missing)
        );
        assert_eq!(
            read_untrusted(&dir.0, 9).expect("io"),
            Err(Rejected::NotRegular),
            "a directory"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_symlinks_fifos_and_hard_links() {
        let dir = Dir::new("special");
        let secret = dir.0.join("secret");
        std::fs::write(&secret, b"labels").expect("write");
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&secret, &link).expect("symlink");
        assert_eq!(
            read_untrusted(&link, 99).expect("io"),
            Err(Rejected::Symlink)
        );

        let hard = dir.0.join("hard");
        std::fs::hard_link(&secret, &hard).expect("hard link");
        assert_eq!(
            read_untrusted(&hard, 99).expect("io"),
            Err(Rejected::Linked)
        );

        let fifo = dir.0.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(made.success());
        let started = std::time::Instant::now();
        assert_eq!(
            read_untrusted(&fifo, 99).expect("io"),
            Err(Rejected::NotRegular)
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "did not block"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_the_opened_file_even_if_the_path_is_replaced() {
        let dir = Dir::new("replace");
        let path = dir.0.join("out");
        std::fs::write(&path, b"original").expect("write");
        let file = open_no_follow(&path).expect("open");
        std::fs::remove_file(&path).expect("unlink");
        std::os::unix::fs::symlink("/etc/hostname", &path).expect("swap in a link");
        assert_eq!(read_capped(file, 99).expect("io"), Ok(b"original".to_vec()));
    }
}
