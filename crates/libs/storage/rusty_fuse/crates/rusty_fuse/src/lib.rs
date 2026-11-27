//! The rusty_fuse port: one inode-based, read-only `Filesystem` trait and the
//! types it speaks.
//!
//! A backend implements [`Filesystem`] once; an OS adapter (FUSE on Linux,
//! ProjFS on Windows, FSKit on macOS) translates kernel requests into calls on
//! it. This crate holds no adapter, makes no OS call and contains no `unsafe`
//! (design.md § Port, DECISIONS D6, D9).
//!
//! Only the read core exists so far. `Writable`, `Projected` and the `mount_*`
//! functions arrive with the adapters that need them.

#![forbid(unsafe_code)]

mod errno;
mod name;
mod types;

pub use errno::Errno;
pub use name::{Name, NAME_MAX};
pub use types::{
    Attr, Cookie, DirEntry, DirSink, Entry, Handle, NodeId, NodeKind, OpenFlags, Ttl, Verifier,
};

/// The read core every backend implements.
///
/// Adapters call it concurrently from several threads, so implementations use
/// interior mutability. Node ids are stable for the life of a mount and the
/// root is [`NodeId::ROOT`]. `lookup` counts one reference on the returned
/// node; `forget` drops `nlookup` of them.
pub trait Filesystem: Send + Sync {
    /// Resolves `name` inside directory `parent`.
    ///
    /// # Errors
    /// `NoEnt` when the name does not exist, `NotDir` when `parent` is not a
    /// directory, `Stale` when `parent` is unknown.
    fn lookup(&self, parent: NodeId, name: Name<'_>) -> Result<Entry, Errno>;

    /// Returns the attributes of `id`.
    ///
    /// # Errors
    /// `Stale` when `id` is unknown, `NoEnt` when it has gone.
    fn getattr(&self, id: NodeId) -> Result<Attr, Errno>;

    /// Pushes the entries of `dir`, starting after `cookie`, into `sink` until
    /// the sink is full or the directory ends. `.` and `..` are not listed;
    /// adapters add them where their OS wants them.
    ///
    /// Returns the directory's current verifier. A caller resuming with a
    /// non-start cookie passes the verifier it was given; when the directory
    /// changed since, the call fails with `Stale` and the caller restarts.
    ///
    /// # Errors
    /// `NotDir`, `Stale` (unknown id or changed directory), `NoEnt`.
    fn readdir(
        &self,
        dir: NodeId,
        cookie: Cookie,
        verifier: Verifier,
        sink: &mut dyn DirSink,
    ) -> Result<Verifier, Errno>;

    /// Opens `id` for reading. The default needs no per-open state.
    ///
    /// # Errors
    /// Backend-specific; a read-only backend refuses write access with `ReadOnly`.
    fn open(&self, id: NodeId, flags: OpenFlags) -> Result<Handle, Errno> {
        let _ = (id, flags);
        Ok(Handle::NONE)
    }

    /// Reads into `buf` from `offset`, returning the bytes read. Zero means
    /// end of file. A short read happens only at end of file.
    ///
    /// # Errors
    /// `IsDir`, `BadHandle`, `Stale`, `Io`.
    fn read(&self, id: NodeId, fh: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, Errno>;

    /// Releases a handle returned by [`Filesystem::open`].
    fn release(&self, id: NodeId, fh: Handle) {
        let _ = (id, fh);
    }

    /// Writes the target of symlink `id` into `buf`, replacing its contents.
    ///
    /// # Errors
    /// `Inval` when `id` is not a symlink (the default).
    fn readlink(&self, id: NodeId, buf: &mut Vec<u8>) -> Result<(), Errno> {
        let _ = (id, buf);
        Err(Errno::Inval)
    }

    /// Drops `nlookup` references taken by [`Filesystem::lookup`].
    fn forget(&self, id: NodeId, nlookup: u64) {
        let _ = (id, nlookup);
    }
}
