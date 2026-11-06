//! Owning a socket's name (C69 slice (a), Orca's O6 rules): a server never
//! unlinks a name it didn't create.
//!
//! [`bind`] binds a fresh temporary name beside the target, then links it
//! to the target, which fails if anything is there. If something is, a
//! live server there makes it refuse; a socket nothing answers on is
//! replaced by renaming the temporary name over it; anything else is left
//! alone and refused. The returned [`Owned`] removes the target only while
//! it still is the socket this server bound.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The socket name a server made, known by its file's identity.
#[derive(Debug)]
pub struct Owned {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl Owned {
    /// Removes the name if it is still this server's socket; a name
    /// someone else has put there since is left alone.
    pub fn release(self) -> io::Result<()> {
        match std::fs::symlink_metadata(&self.path) {
            Ok(m) if m.dev() == self.dev && m.ino() == self.ino => std::fs::remove_file(&self.path),
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Binds `socket` under `umask` (so the socket has its mode as it is made,
/// with no chmod to follow a swapped path), following the module's rules.
/// Call it before starting any thread: the umask is the process's.
pub fn bind(socket: &Path, umask: u32) -> io::Result<(UnixListener, Owned)> {
    let dir = socket
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = dir.join(format!(".rusty-mux.{}", std::process::id()));
    // Our own name: one left by a dead process with this pid is stale.
    remove_if_present(&temp)?;
    let old = rusty_libc::umask::umask(umask);
    let bound = UnixListener::bind(&temp);
    rusty_libc::umask::umask(old);
    let listener = bound?;
    let placed = place(&temp, socket);
    // After a link the temporary name is a second name; after a rename or
    // a refusal it is ours to clear either way.
    remove_if_present(&temp)?;
    let meta = placed.and_then(|()| std::fs::symlink_metadata(socket))?;
    let owned = Owned {
        path: socket.to_owned(),
        dev: meta.dev(),
        ino: meta.ino(),
    };
    Ok((listener, owned))
}

/// Gives `temp`'s socket the name `socket`.
fn place(temp: &Path, socket: &Path) -> io::Result<()> {
    match std::fs::hard_link(temp, socket) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        linked => return linked,
    }
    let shown = socket.display();
    let is_socket = std::fs::symlink_metadata(socket)?.file_type().is_socket();
    if !is_socket {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{shown} exists and is not a socket; leaving it alone"),
        ));
    }
    match UnixStream::connect(socket) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("a server already answers at {shown}"),
        )),
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => std::fs::rename(temp, socket),
        Err(e) => Err(io::Error::new(e.kind(), format!("probing {shown}: {e}"))),
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Makes `dir` this user's private directory (0700) if it is missing, and
/// checks it either way: each part as [`check_parts`] requires, the last
/// this user's and open to no one else. Its parent must exist.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    let me = rusty_libc::process::geteuid();
    check_parts(dir, me, true)?;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    check_parts(dir, me, false)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if meta.uid() != me || meta.mode() & 0o077 != 0 {
        return Err(unsafe_dir(dir, "is not this user's alone".into()));
    }
    Ok(())
}

/// Checks each part of `dir`: absolute, a real directory (no symbolic
/// link), root's or this user's, and writable by others only when sticky
/// (as `/tmp`) and not `dir` itself. A part that does not exist passes
/// only when `allow_missing`.
pub fn check_parts(dir: &Path, me: u32, allow_missing: bool) -> io::Result<()> {
    if !dir.is_absolute() {
        return Err(unsafe_dir(dir, "is not an absolute path".into()));
    }
    for part in dir.ancestors() {
        let meta = match std::fs::symlink_metadata(part) {
            Ok(meta) => meta,
            Err(e) if allow_missing && e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let shown = part.display();
        if !meta.is_dir() {
            return Err(unsafe_dir(
                dir,
                format!("goes through {shown}, not a directory"),
            ));
        }
        let owner_ok = meta.uid() == 0 || meta.uid() == me;
        let sticky = meta.mode() & 0o1000 != 0;
        if !owner_ok || (meta.mode() & 0o022 != 0 && (part == dir || !sticky)) {
            return Err(unsafe_dir(
                dir,
                format!("goes through {shown}, which another user could change"),
            ));
        }
    }
    Ok(())
}

pub fn unsafe_dir(dir: &Path, why: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{}: {why}", dir.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rmx-endpoint-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("scratch");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        dir
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("stat").mode() & 0o777
    }

    fn only_entry(dir: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name())
            .collect()
    }

    #[test]
    fn a_free_name_is_bound_with_the_umask_mode_and_no_temporary_left() {
        let dir = scratch("free");
        let socket = dir.join("s.sock");
        let (listener, owned) = bind(&socket, 0o177).expect("bind");
        assert_eq!(mode(&socket), 0o600);
        assert_eq!(only_entry(&dir), vec![std::ffi::OsString::from("s.sock")]);
        UnixStream::connect(&socket).expect("answers");
        drop(listener);
        owned.release().expect("release");
        assert!(!socket.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_server_is_refused_and_its_name_kept() {
        let dir = scratch("live");
        let socket = dir.join("s.sock");
        let (_first, _owned) = bind(&socket, 0o177).expect("first");
        let err = bind(&socket, 0o177).expect_err("second refused");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        UnixStream::connect(&socket).expect("the first still answers");
        assert_eq!(only_entry(&dir).len(), 1, "no temporary left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let dir = scratch("stale");
        let socket = dir.join("s.sock");
        drop(UnixListener::bind(&socket).expect("stale"));
        let (_listener, _owned) = bind(&socket, 0o177).expect("replaces");
        UnixStream::connect(&socket).expect("answers");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_socket_is_left_alone() {
        let dir = scratch("file");
        let socket = dir.join("s.sock");
        std::fs::write(&socket, b"mine").expect("file");
        let err = bind(&socket, 0o177).expect_err("refused");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&socket).expect("kept"), b"mine");
        assert_eq!(only_entry(&dir).len(), 1, "no temporary left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_leaves_a_successors_socket() {
        let dir = scratch("successor");
        let socket = dir.join("s.sock");
        let (first, owned) = bind(&socket, 0o177).expect("first");
        drop(first);
        let (_second, _next) = bind(&socket, 0o177).expect("successor");
        owned.release().expect("release");
        UnixStream::connect(&socket).expect("the successor's name stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_dir_is_made_0700_and_an_open_one_refused() {
        let dir = scratch("private");
        let made = dir.join("rusty_mux");
        private_dir(&made).expect("made");
        assert_eq!(mode(&made), 0o700);
        private_dir(&made).expect("again");
        std::fs::set_permissions(&made, std::fs::Permissions::from_mode(0o750)).expect("chmod");
        assert!(private_dir(&made).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
