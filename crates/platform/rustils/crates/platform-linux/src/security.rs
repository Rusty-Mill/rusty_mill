//! `Csprng`/`CredentialStore`/`Sandbox` trait impls over the sys layer.
//! No `unsafe` here.

use std::path::Path;

use platform::error::Result;
use platform::security::{
    CredentialStore, CredentialStoreStatus, Csprng, Sandbox, SandboxStatus, TrustAnchors,
};

use crate::sys::secret_service;
use crate::sys::security as syssec;
use crate::sys::trust_anchors;

/// The Linux backend's [`Csprng`] capability. Stateless — every call is
/// a fresh `getrandom(2)` syscall, mirroring [`crate::LinuxNet`].
pub struct LinuxCsprng;

impl Csprng for LinuxCsprng {
    fn fill_random(&self, buf: &mut [u8]) -> Result<()> {
        syssec::fill_random(buf)
    }
}

/// The Linux backend's [`TrustAnchors`] capability (rustils#88).
/// Stateless — every call re-probes and re-reads, mirroring
/// [`LinuxCsprng`]. Deliberately uncached: a system that has just had
/// `update-ca-certificates` run on it should not keep serving the old
/// anchor set for the life of the process, and callers load anchors once
/// at TLS-config construction rather than per connection.
pub struct LinuxTrustAnchors;

impl TrustAnchors for LinuxTrustAnchors {
    fn load_anchors(&self) -> Result<Vec<Vec<u8>>> {
        trust_anchors::load_anchors()
    }
}

/// The Linux backend's [`CredentialStore`] capability (rustils#78): the
/// Secret Service API (`org.freedesktop.secrets`) over `sys::dbus`'s
/// hand-rolled transport (rustils#77) — see `sys::secret_service`'s own
/// doc comment for the full reachability/error-shape contract. Stateless
/// — every call opens a fresh D-Bus connection and Secret Service
/// session, mirroring how [`LinuxCsprng`]/[`LinuxSandbox`] hold no state
/// either.
pub struct LinuxCredentialStore;

impl CredentialStore for LinuxCredentialStore {
    fn available(&self) -> CredentialStoreStatus {
        secret_service::available()
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>> {
        secret_service::get(service, account)
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<()> {
        secret_service::set(service, account, secret)
    }
}

/// The Linux backend's [`Sandbox`] capability: Landlock for
/// [`Sandbox::confine_filesystem`], seccomp-BPF for
/// [`Sandbox::block_inet_sockets`]. Stateless — every call is a fresh
/// set of syscalls against the calling thread, mirroring
/// [`crate::LinuxNet`]/[`LinuxCsprng`].
pub struct LinuxSandbox;

impl Sandbox for LinuxSandbox {
    fn confine_filesystem(
        &self,
        readable_roots: &[&Path],
        writable_roots: &[&Path],
    ) -> Result<SandboxStatus> {
        syssec::confine_filesystem(readable_roots, writable_roots)
    }

    fn block_inet_sockets(&self) -> Result<SandboxStatus> {
        syssec::block_inet_sockets()
    }
}
