//! `Csprng`/`CredentialStore`/`Sandbox` trait impls over the sys layer.
//! No `unsafe` here.

use std::ffi::OsStr;
use std::path::Path;

use platform::error::Result;
use platform::security::{
    CredentialStore, CredentialStoreStatus, Csprng, Sandbox, SandboxStatus, TrustAnchors,
};

use crate::sys::security as syssec;

/// The Windows backend's [`Csprng`] capability. Stateless — every call
/// is a fresh `BCryptGenRandom` call, mirroring [`crate::WindowsNet`].
pub struct WindowsCsprng;

impl Csprng for WindowsCsprng {
    fn fill_random(&self, buf: &mut [u8]) -> Result<()> {
        syssec::fill_random(buf)
    }
}

/// The Windows backend's [`TrustAnchors`] capability (rustils#88): the
/// system ROOT certificate store, enumerated. Stateless — every call
/// reopens and re-enumerates, mirroring [`WindowsCsprng`], so a machine
/// that has just had a root installed doesn't keep serving the old set
/// for the life of the process.
///
/// Carries Windows' lazy-population fidelity limit; see
/// [`TrustAnchors`] and `sys::security::load_anchors`.
pub struct WindowsTrustAnchors;

impl TrustAnchors for WindowsTrustAnchors {
    fn load_anchors(&self) -> Result<Vec<Vec<u8>>> {
        syssec::load_anchors()
    }
}

/// The Windows backend's [`CredentialStore`] capability (rustils#76):
/// Credential Manager's `CRED_TYPE_GENERIC` slot. Stateless — every call
/// is a fresh `CredWriteW`/`CredReadW`, mirroring [`WindowsCsprng`].
/// Always [`CredentialStoreStatus::Available`]: Credential Manager is a
/// core OS service present on every supported Windows version, with no
/// "not running" state the way a D-Bus-mediated Linux service has.
pub struct WindowsCredentialStore;

impl CredentialStore for WindowsCredentialStore {
    fn available(&self) -> CredentialStoreStatus {
        CredentialStoreStatus::Available
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>> {
        syssec::credential_get(&credential_target_name(service, account))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<()> {
        let target = credential_target_name(service, account);
        syssec::credential_set(&target, OsStr::new(account), secret)
    }
}

/// Credential Manager's identity key for a stored credential is
/// `(TargetName, Type)` alone — `UserName` is a display field, not part
/// of it, so `CredWriteW` under the same `TargetName` silently replaces
/// *any* existing credential there regardless of its `UserName`. Folding
/// `account` into `TargetName` is therefore required, not a style choice,
/// for two different accounts under the same `service` to coexist as two
/// distinct Credential Manager entries rather than clobbering each
/// other. `\u{1}` (SOH) separates the two: a control character no real
/// service/account name is realistically going to contain, keeping the
/// composition unambiguous without a general escaping scheme.
fn credential_target_name(service: &str, account: &str) -> std::ffi::OsString {
    std::ffi::OsString::from(format!("rustils\u{1}{service}\u{1}{account}"))
}

/// The Windows backend's [`Sandbox`] capability. No confinement
/// mechanism exists here yet (restricted tokens/AppContainer have no
/// donor and don't obviously fit an arbitrary, non-packaged process —
/// see `docs/design-discussion-sandbox.md`); every call reports
/// [`SandboxStatus::Unsupported`], never silently claims enforcement.
pub struct WindowsSandbox;

impl Sandbox for WindowsSandbox {
    fn confine_filesystem(
        &self,
        _readable_roots: &[&Path],
        _writable_roots: &[&Path],
    ) -> Result<SandboxStatus> {
        Ok(SandboxStatus::Unsupported)
    }

    fn block_inet_sockets(&self) -> Result<SandboxStatus> {
        Ok(SandboxStatus::Unsupported)
    }
}
