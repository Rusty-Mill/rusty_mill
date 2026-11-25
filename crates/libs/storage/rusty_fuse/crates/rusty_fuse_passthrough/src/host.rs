//! Host-filesystem translation: names to path components, metadata to `Attr`,
//! `io::Error` to `Errno`, and positional reads. All OS differences live here.

use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata};
use std::io;
use std::path::{Component, Path};
use std::time::SystemTime;

use rusty_fuse::{Attr, Errno, Name, NodeKind};

/// Turns a port name into one host path component.
///
/// # Errors
/// `Inval` when the host cannot hold the name as a single plain component:
/// on Windows, non-UTF-8 bytes, `\`, `:` (alternate data streams) or a drive
/// prefix; anywhere, anything that does not parse as exactly one normal
/// component. This is what keeps a lookup inside the mirrored root.
pub(crate) fn component(name: Name<'_>) -> Result<OsString, Errno> {
    let os = to_os(name.as_bytes())?;
    if cfg!(windows) && name.as_bytes().iter().any(|&b| b == b'\\' || b == b':') {
        return Err(Errno::Inval);
    }
    let mut parts = Path::new(&os).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(part)), None) if part == os.as_os_str() => Ok(os),
        _ => Err(Errno::Inval),
    }
}

#[cfg(unix)]
fn to_os(bytes: &[u8]) -> Result<OsString, Errno> {
    use std::os::unix::ffi::OsStrExt;
    Ok(OsStr::from_bytes(bytes).to_os_string())
}

#[cfg(not(unix))]
fn to_os(bytes: &[u8]) -> Result<OsString, Errno> {
    std::str::from_utf8(bytes)
        .map(OsString::from)
        .map_err(|_| Errno::Inval)
}

/// The bytes of a host name, or `None` when the port cannot carry it back
/// (on Windows, a name that is not valid Unicode).
pub(crate) fn name_bytes(os: &OsStr) -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(os.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        os.to_str().map(|s| s.as_bytes().to_vec())
    }
}

/// The node kind of host metadata read without following symlinks, or `None`
/// for kinds the mirror does not serve (FIFOs, sockets, devices).
pub(crate) fn kind(meta: &Metadata) -> Option<NodeKind> {
    let ft = meta.file_type();
    if ft.is_symlink() {
        Some(NodeKind::Symlink)
    } else if ft.is_dir() {
        Some(NodeKind::Dir)
    } else if ft.is_file() {
        Some(NodeKind::File)
    } else {
        None
    }
}

/// Port attributes for host metadata of kind `kind`.
pub(crate) fn attr(meta: &Metadata, kind: NodeKind) -> Attr {
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut attr = Attr {
        kind,
        size: meta.len(),
        perm: 0,
        nlink: 1,
        uid: 0,
        gid: 0,
        mtime,
        ctime: mtime,
    };
    fill_owner_and_mode(meta, &mut attr);
    attr
}

#[cfg(unix)]
fn fill_owner_and_mode(meta: &Metadata, attr: &mut Attr) {
    use std::os::unix::fs::MetadataExt;
    attr.perm = (meta.mode() & 0o7777) as u16;
    attr.nlink = u32::try_from(meta.nlink()).unwrap_or(u32::MAX);
    attr.uid = meta.uid();
    attr.gid = meta.gid();
    let secs = u64::try_from(meta.ctime()).unwrap_or(0);
    let nanos = u32::try_from(meta.ctime_nsec()).unwrap_or(0);
    attr.ctime = SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos);
}

/// Windows and other hosts have no POSIX mode: derive one from the
/// read-only attribute; owner ids stay 0 and `ctime` is `mtime`.
#[cfg(not(unix))]
fn fill_owner_and_mode(meta: &Metadata, attr: &mut Attr) {
    let exec = if attr.kind == NodeKind::File {
        0
    } else {
        0o111
    };
    let write = if meta.permissions().readonly() {
        0
    } else {
        0o200
    };
    attr.perm = 0o444 | write | exec;
}

/// Maps a host I/O error to the port's `Errno`.
pub(crate) fn errno(err: &io::Error) -> Errno {
    match err.kind() {
        io::ErrorKind::NotFound => Errno::NoEnt,
        io::ErrorKind::PermissionDenied => Errno::Access,
        io::ErrorKind::NotADirectory => Errno::NotDir,
        io::ErrorKind::IsADirectory => Errno::IsDir,
        io::ErrorKind::InvalidInput => Errno::Inval,
        io::ErrorKind::ReadOnlyFilesystem => Errno::ReadOnly,
        io::ErrorKind::Unsupported => Errno::NotSupported,
        _ => Errno::Io,
    }
}

/// Reads at `offset` without moving a shared cursor, filling `buf` until it
/// is full or the file ends.
pub(crate) fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    // Host offsets are signed 64-bit; no file reaches past `i64::MAX`, so a
    // read starting there is past end of file (Windows rejects it otherwise).
    const MAX_OFFSET: u64 = i64::MAX as u64;
    let mut done = 0;
    while done < buf.len() {
        let at = offset.saturating_add(done as u64);
        if at > MAX_OFFSET {
            break;
        }
        let n = match read_once(file, at, &mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        done += n;
    }
    Ok(done)
}

#[cfg(unix)]
fn read_once(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn read_once(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    match file.seek_read(buf, offset) {
        // Windows reports a read starting past end of file as an error.
        Err(e) if e.raw_os_error() == Some(38) => Ok(0), // ERROR_HANDLE_EOF
        other => other,
    }
}

#[cfg(not(any(unix, windows)))]
fn read_once(mut file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    file.read(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Name<'_> {
        Name::try_from(s).expect("valid port name")
    }

    #[test]
    fn plain_names_become_one_component() {
        assert_eq!(
            component(name("hello.txt")),
            Ok(OsString::from("hello.txt"))
        );
        assert_eq!(component(name("...")), Ok(OsString::from("...")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_separators_streams_and_prefixes() {
        for bad in ["a\\b", "..\\x", "file:stream", "C:", "\\"] {
            assert_eq!(component(name(bad)), Err(Errno::Inval), "{bad}");
        }
        let lone = Name::new(&[0xff]).expect("valid port name");
        assert_eq!(component(lone), Err(Errno::Inval), "non-UTF-8 on Windows");
    }

    #[cfg(unix)]
    #[test]
    fn unix_keeps_backslashes_and_raw_bytes() {
        assert!(component(name("a\\b")).is_ok());
        assert!(component(Name::new(&[0xff]).expect("valid port name")).is_ok());
    }

    #[test]
    fn io_errors_map_to_errno() {
        let cases = [
            (io::ErrorKind::NotFound, Errno::NoEnt),
            (io::ErrorKind::PermissionDenied, Errno::Access),
            (io::ErrorKind::NotADirectory, Errno::NotDir),
            (io::ErrorKind::Other, Errno::Io),
        ];
        for (kind, expected) in cases {
            assert_eq!(errno(&io::Error::from(kind)), expected, "{kind:?}");
        }
    }
}
