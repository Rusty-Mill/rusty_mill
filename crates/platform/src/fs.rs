//! Capability-style filesystem traits (RFC v2 §5.3, decision D-6).
//!
//! There are deliberately no global path functions here. All operations are
//! relative to a [`Dir`] handle opened once — mapping to the `openat` family
//! on Linux and handle-relative opens on Windows. This shape exists for
//! three reasons (RFC v2 §5.3): TOCTOU hygiene, direct support for a
//! consumer-maintained virtual cwd (rush subshells), and because handle-
//! relative NT opens are among the most instructive Windows topics for the
//! project's understanding mandate (M1).
//!
//! Paths and names are `OsStr`/`OsString` — never `str` (RFC v2 §5.2):
//! unix names are bytes, and a `str` surface makes correct behavior on
//! non-UTF-8 names unrepresentable.

use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::error::{ErrorKind, OsCode, PlatformError, Result};

/// Options for opening or creating a file relative to a [`Dir`].
///
/// Mirrors the intersection of `openat` flags and `CreateFileW` dispositions
/// that both backends can honor; per-OS extensions live on the backend
/// types, not here.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub create: bool,
    pub create_new: bool,
    pub truncate: bool,
}

impl OpenOptions {
    pub fn read() -> Self {
        Self {
            read: true,
            ..Self::default()
        }
    }

    pub fn create_truncate() -> Self {
        Self {
            write: true,
            create: true,
            truncate: true,
            ..Self::default()
        }
    }
}

/// Access-mode bits for [`Dir::access`] — mirrors POSIX `faccessat`'s
/// `R_OK`/`W_OK`/`X_OK`. `F_OK` (existence) has no field here: it's
/// already [`Dir::metadata`]'s job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccessMode {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl AccessMode {
    pub fn read() -> Self {
        Self {
            read: true,
            ..Self::default()
        }
    }

    pub fn write() -> Self {
        Self {
            write: true,
            ..Self::default()
        }
    }

    pub fn execute() -> Self {
        Self {
            execute: true,
            ..Self::default()
        }
    }
}

/// Metadata for a filesystem entry.
///
/// `nlink`/`modified` (`ls -l`'s donor material, coreutils gap backlog
/// #63) are portable across both backends — unlike [`UnixMode`]'s
/// special bits, hard-link counts and modification times are concepts
/// both Linux (`st_nlink`/`st_mtime`) and Windows
/// (`FILE_STANDARD_INFO::NumberOfLinks`/`FILE_BASIC_INFO::LastWriteTime`)
/// genuinely have, so there is no `Option` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub file_type: FileType,
    pub len: u64,
    pub nlink: u64,
    pub modified: SystemTime,
}

/// POSIX mode bits and ownership (`test -u/-g/-k/-O/-G`'s donor material,
/// D11) — `setuid`/`setgid`/`sticky`, the standard `rwx` `permissions`
/// bits (coreutils gap backlog #64's read side; `0o777`-masked, e.g.
/// `0o755`), and the owning `uid`/`gid`. Windows has no analog for any
/// of this (NTFS security descriptors are a wholly different model,
/// not a POSIX-mode-bit superset); [`Dir::unix_mode`] returns `Ok(None)`
/// there rather than fabricating zeroed-out values — "this OS has no
/// such concept" is a real answer, not an error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnixMode {
    pub setuid: bool,
    pub setgid: bool,
    pub sticky: bool,
    pub uid: u32,
    pub gid: u32,
    pub permissions: u16,
}

/// The `chmod`-settable subset of [`UnixMode`]: the standard `rwx`
/// permission bits and the `setuid`/`setgid`/`sticky` special bits
/// (coreutils gap backlog #64 — [`Dir::unix_mode`]'s write-side
/// companion). Deliberately excludes `uid`/`gid`: those are `chown`'s
/// job, a distinct syscall class this method does not touch, and
/// reusing [`UnixMode`] itself here would let a caller populate
/// `uid`/`gid` and have them silently ignored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mode {
    pub setuid: bool,
    pub setgid: bool,
    pub sticky: bool,
    pub permissions: u16,
}

/// An opaque per-OS file identity, equality-comparable only (`test -ef`'s
/// donor material, D11) — POSIX's `(dev, ino)` pair on Linux, `(volume
/// serial, file index)` on Windows via `GetFileInformationByHandle`. Two
/// [`FileId`]s are equal exactly when they name the same underlying file
/// object, which — unlike [`UnixMode`] — both backends can answer, so
/// there is no `Option` here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u64, pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileType {
    File,
    Dir,
    Symlink,
    Other,
}

/// A directory entry yielded by [`Dir::read_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Entry name — a single component, not a path.
    pub name: OsString,
    pub file_type: FileType,
}

/// An open file. Object-safe; backends return `Box<dyn File>`.
pub trait File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn flush(&mut self) -> Result<()>;

    /// Durability: block until this file's writes are on stable storage
    /// (`fsync`/`FlushFileBuffers`), not merely past the OS page/write
    /// cache. `flush` above is not this — a synchronous `write` already
    /// has no userspace buffer to flush; this is the distinct, explicit
    /// operation `flush`'s doc comment always said would come when a
    /// consumer needed it. [`Dir::write_atomic`] (D11, convergence
    /// roadmap Phase 3) is that consumer.
    fn sync_all(&mut self) -> Result<()>;

    /// Duplicate this file's underlying OS handle (`dup(2)`/
    /// `DuplicateHandle`) into a fresh, independent `File` that shares
    /// the *same open-file description* — position included: a read or
    /// write through either handle advances the other's next
    /// read/write position too. That sharing is the entire point (it is
    /// what `dup` gives and a fresh [`Dir::open`] of the same path does
    /// not): it is what lets a caller build a `2>&1`/`&> file`-style
    /// shell redirect — both `stdout` and `stderr` wired to
    /// [`crate::process::Stdio::File`] instances that share one
    /// underlying description, exactly like a real shell's `dup2`-based
    /// redirect does (RFC v2 D5 — `rush/src/exec.rs`'s `Redirect::Dup`).
    /// The clone is not inheritable by a child process on its own (Unix:
    /// `CLOEXEC` set; Windows: not marked inheritable) — inheritance is
    /// [`crate::process::Stdio::File`]'s job at spawn time, not this
    /// method's.
    fn try_clone(&self) -> Result<Box<dyn File>>;

    /// Downcast hook, mirroring [`crate::process::Child::as_any_mut`] —
    /// lets a backend's `Spawner::spawn` recover its own concrete
    /// `File` type (`LinuxFile`/`WindowsFile`) from a
    /// [`crate::process::Stdio::File`]'s object-safe `Box<dyn File>` to
    /// reach the raw fd/handle a spawn-time `dup2`/`DuplicateHandle`
    /// wiring needs. Not for consumers.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// An open directory: the capability all filesystem operations flow through.
///
/// Object-safe. `&self` receivers throughout: a `Dir` is a capability that
/// may be shared, and backends manage any interior synchronization their OS
/// primitives require.
pub trait Dir {
    /// Open a file at `rel` (a relative path) under this directory.
    fn open(&self, rel: &OsStr, opts: &OpenOptions) -> Result<Box<dyn File>>;

    /// Open a subdirectory as a new capability.
    fn open_dir(&self, rel: &OsStr) -> Result<Box<dyn Dir>>;

    /// Create a subdirectory.
    fn create_dir(&self, rel: &OsStr) -> Result<()>;

    /// Metadata for the entry at `rel`.
    fn metadata(&self, rel: &OsStr) -> Result<Metadata>;

    /// Probe whether every bit set in `mode` is permitted for `rel`
    /// (POSIX `faccessat(2)`, real — not effective — uid/gid: the plain
    /// syscall's check, not the glibc-only `AT_EACCESS` emulation, kept
    /// consistent with what Track P's `rusty_libc::fs::faccessat` can
    /// support; see `fn access` in each backend's `sys/` module for the
    /// rationale). `Err(PermissionDenied)` if any requested bit is
    /// refused; other failures (e.g. `NotFound`) surface as themselves.
    /// Existence alone is [`Dir::metadata`]'s job, not this one's — an
    /// empty `mode` is a vacuous "yes."
    ///
    /// Follows a terminal symlink, like `open` and unlike `metadata`.
    ///
    /// Windows divergence (`docs/divergences.md` #005): no single
    /// syscall answers this, and regular files have no execute-
    /// permission bit at all (execute is a property of file type, not
    /// an ACL check any consumer code inspects) — `mode.execute` is
    /// therefore always granted once the entry is confirmed to exist,
    /// the same behavior every practical Windows `access()`/`_waccess`
    /// implementation gives. `read`/`write` are answered by a trial
    /// open with the matching access mask, immediately closed — the
    /// actual operation this probe predicts, not a separate ACL query
    /// that could disagree with it.
    fn access(&self, rel: &OsStr, mode: AccessMode) -> Result<()>;

    /// [`UnixMode`] for `rel`, or `Ok(None)` on a backend with no such
    /// concept (Windows). Does not follow a terminal symlink, matching
    /// [`Dir::metadata`]'s lstat-style contract — the same object, not
    /// its target.
    fn unix_mode(&self, rel: &OsStr) -> Result<Option<UnixMode>>;

    /// Set the `setuid`/`setgid`/`sticky` special bits and the standard
    /// `rwx` permission bits at `rel` (POSIX `fchmodat(2)` — `chmod`'s
    /// write-side counterpart to [`Dir::unix_mode`]'s read side;
    /// coreutils gap backlog #64). Owning `uid`/`gid` is `chown`'s job,
    /// a distinct syscall class not covered here.
    ///
    /// Follows a terminal symlink — unlike [`Dir::unix_mode`]/
    /// [`Dir::metadata`]: Linux's kernel does not implement changing a
    /// symlink's own permissions at all (`fchmodat(...,
    /// AT_SYMLINK_NOFOLLOW)` fails `ENOTSUP` on every real filesystem,
    /// since symlink permission bits are unused and ignored) — this
    /// method changes the **target**'s mode, matching `chmod(1)`'s own
    /// behavior when pointed at a symlink.
    ///
    /// `Err` with `ErrorKind::Unsupported` on a backend with no POSIX
    /// mode-bit concept (Windows — `docs/divergences.md` #009) — never
    /// a silent no-op: unlike a best-effort hardening step, a caller
    /// here explicitly asked to change permissions, so pretending
    /// success would misrepresent what happened.
    fn set_unix_mode(&self, rel: &OsStr, mode: Mode) -> Result<()>;

    /// [`FileId`] for `rel` — same-file identity, `test -ef`'s donor
    /// material. Does not follow a terminal symlink, matching
    /// [`Dir::metadata`].
    fn file_id(&self, rel: &OsStr) -> Result<FileId>;

    /// List this directory's entries.
    ///
    /// Order is unspecified and differs across backends; consumers that
    /// need determinism sort. (Pinned by behavior spec `docs/behavior/fs.md`.)
    fn read_dir(&self) -> Result<Vec<DirEntry>>;

    /// Remove the file at `rel`.
    fn remove_file(&self, rel: &OsStr) -> Result<()>;

    /// Remove the (empty) directory at `rel`.
    fn remove_dir(&self, rel: &OsStr) -> Result<()>;

    /// Create `link_name` (relative to this directory) as a symbolic
    /// link whose stored target is `target`, byte-for-byte (POSIX
    /// `symlinkat(2)`; D11, convergence roadmap symlink slice). Fails
    /// `AlreadyExists` if `link_name` already names an entry.
    ///
    /// `target` is opaque content, not validated or resolved against
    /// this directory: it need not exist, and if it is a relative
    /// string, it resolves (when the OS later follows the link)
    /// relative to `link_name`'s own directory, not to `self` at the
    /// time of this call. A leading `/` or a Windows drive/UNC prefix
    /// is treated as absolute; anything else is stored as a
    /// relative target.
    ///
    /// Windows divergence (`docs/divergences.md`): unlike POSIX, the NT
    /// reparse point backing a symlink must declare at creation time
    /// whether it names a file or a directory — there is no single
    /// reparse tag that means "either." This backend decides by
    /// best-effort `metadata`-ing `target` relative to `self`: an
    /// existing directory there makes a directory-type link, anything
    /// else (a file, or nothing resolvable — a dangling link, an
    /// absolute target, a target elsewhere) makes a file-type link. A
    /// dangling link later satisfied by a directory stays a file-type
    /// link on Windows until recreated; Linux has no such distinction.
    fn symlink(&self, target: &OsStr, link_name: &OsStr) -> Result<()>;

    /// Read the stored target of the symlink at `rel` (POSIX
    /// `readlinkat(2)`) — the same bytes `symlink` was given, not
    /// resolved or validated against the filesystem. `rel` must itself
    /// be a symlink (`InvalidInput` otherwise); see
    /// [`Dir::metadata`]'s [`FileType::Symlink`] to check first.
    fn read_link(&self, rel: &OsStr) -> Result<OsString>;

    /// Rename `from` to `to`, both relative to this directory,
    /// **replacing** `to` if it already exists (POSIX `rename(2)` /
    /// `renameat2` with no flags; Windows `FILE_RENAME_INFO` with
    /// `ReplaceIfExists`). Atomic: `to` is never observably absent —
    /// concurrent readers see either the old file or the new one.
    fn rename(&self, from: &OsStr, to: &OsStr) -> Result<()>;

    /// Rename `from` to `to`, refusing (`AlreadyExists`) if `to` already
    /// exists, instead of replacing it (`RENAME_NOREPLACE` /
    /// `ReplaceIfExists = false`) — the check-and-rename happens
    /// atomically in the kernel, so this is race-free where a
    /// stat-then-rename from the consumer would not be.
    fn rename_no_replace(&self, from: &OsStr, to: &OsStr) -> Result<()>;

    /// Durably write `contents` to `rel`, atomically: never leaves a
    /// partially-written or missing file observable at that name, even
    /// across a crash between the write and the rename that publishes
    /// it (D11, convergence roadmap Phase 3 — the pattern independently
    /// present in nexus's `storage/atomic.rs` and rusty_naner's staged
    /// install). Default-provided: it composes [`open`](Dir::open) +
    /// [`File::write`] + [`File::sync_all`] + [`rename`](Dir::rename),
    /// so every backend gets it for free and there is exactly one
    /// implementation to trust.
    ///
    /// Sequence: write into a same-directory temp name (guaranteeing
    /// the final rename is same-filesystem, hence atomic) → `sync_all`
    /// the temp file (durability *before* the rename, not after — a
    /// crash before this point leaves only the temp name, never a
    /// half-written `rel`) → close it → `rename` over `rel`. The temp
    /// file is best-effort removed if the write/sync step fails.
    fn write_atomic(&self, rel: &OsStr, contents: &[u8]) -> Result<()> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut tmp_name = rel.to_os_string();
        tmp_name.push(format!(".rustils-tmp-{}-{n:x}", std::process::id()));

        let write_and_sync = || -> Result<()> {
            let mut f = self.open(
                &tmp_name,
                &OpenOptions {
                    write: true,
                    create_new: true,
                    ..OpenOptions::default()
                },
            )?;
            let mut off = 0usize;
            while off < contents.len() {
                let n = f.write(&contents[off..])?;
                if n == 0 {
                    return Err(
                        PlatformError::new(ErrorKind::Other, OsCode::None, "write_atomic")
                            .with_path(tmp_name.as_os_str()),
                    );
                }
                off += n;
            }
            f.sync_all()
        };

        if let Err(e) = write_and_sync() {
            let _ = self.remove_file(&tmp_name);
            return Err(e);
        }
        self.rename(&tmp_name, rel)
    }
}

/// A source of anonymous, memory-backed files with no filesystem
/// namespace entry at all (`memfd_create` — D11, landed independently
/// per `docs/decision-request-fork-execve.md`'s option 3: the
/// thread-free here-doc mechanism D4 cites as the invariant that would
/// make a raw `clone(SIGCHLD)` fork sound, useful on its own regardless
/// of whether that larger question is ever resolved toward raw).
///
/// Deliberately **not** a [`Dir`] method: every other operation in this
/// module is `rel`-relative, resolved against a directory capability
/// (RFC v2 §5.3's whole point). `memfd_create` takes no path and
/// touches no directory at all — there is no `rel` to resolve, so
/// putting it on `Dir` would mean an unused capability receiver on
/// every call, misrepresenting the capability-rooted discipline the
/// rest of this trait exists to enforce. A separate, small trait
/// (mirroring [`crate::security::Csprng`]'s own shape: one method, no
/// capability it doesn't need) keeps that boundary honest.
pub trait AnonymousFile {
    /// Create a fresh anonymous file, unlinked from any directory from
    /// the moment it exists — readable/writable like an ordinary
    /// [`File`], but never visible in any directory listing
    /// ([`Dir::read_dir`]) and never outliving every open handle to it
    /// (no [`Dir::remove_file`] needed or possible). `name` is a
    /// debugging label only (Linux: visible in `/proc/self/fd/<n>`),
    /// not a path — it names nothing a second call or another process
    /// could look up, and need not be unique.
    fn create_memfd(&self, name: &str) -> Result<Box<dyn File>>;
}
