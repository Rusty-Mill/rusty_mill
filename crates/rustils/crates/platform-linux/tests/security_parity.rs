//! Security parity suite (RFC v2 R5+, D15) for the Linux backend.
//!
//! The assertion sets live in `platform-parity`; this file records only
//! which of them apply here, plus this backend's own expectations.
//! Mock conformance moved to `platform-mock/tests/parity_conformance.rs`
//! when the sets were extracted — it was running identically in three
//! backend suites.

#![cfg(target_os = "linux")]

use platform::error::ErrorKind;
use platform::security::{CredentialStore, CredentialStoreStatus, TrustAnchors};
use platform_parity::security::{assert_csprng_behavior, assert_trust_anchors_behavior};

#[test]
fn linux_csprng_conforms() {
    assert_csprng_behavior(&platform_linux::LinuxCsprng);
}

/// rustils#78's real Linux backend (Secret Service over D-Bus), run in
/// this suite's own environment where no D-Bus session bus is reachable
/// (no `DBUS_SESSION_BUS_ADDRESS`, no daemon running): `available()`
/// reports `Unavailable` — a real mechanism exists on this OS, it's just
/// not reachable right now — and `get`/`set` surface that as a real
/// `Err` rather than a silent `Ok(None)`/`Ok(())`, per the trait's own
/// contract (a clean miss and "the store isn't reachable" are different
/// claims). Live round-trip coverage against a real, reachable Secret
/// Service lives in `tests/secret_service.rs`, which spawns its own
/// `dbus-daemon`/`gnome-keyring-daemon` pair.
///
/// Backend-specific by nature, so it stays here rather than moving into
/// the shared sets: "no bus reachable" is not a state Windows or macOS
/// has.
#[test]
fn linux_credential_store_reports_unavailable_with_no_bus_reachable() {
    let store = platform_linux::LinuxCredentialStore;
    assert_eq!(store.available(), CredentialStoreStatus::Unavailable);
    assert!(store.get("svc", "acct").is_err());
    assert!(store.set("svc", "acct", b"secret").is_err());
}

/// rustils#88's real Linux backend against this machine's actual trust
/// store. Skips rather than fails where no store exists — a minimal
/// container image legitimately has none, and the backend's documented
/// answer there is `NotFound`, which is not a bug to fail CI over.
#[test]
fn linux_trust_anchors_conforms() {
    let anchors = platform_linux::LinuxTrustAnchors;
    match anchors.load_anchors() {
        Ok(_) => assert_trust_anchors_behavior(&anchors),
        Err(e) => {
            assert_eq!(
                e.kind,
                ErrorKind::NotFound,
                "a host with no trust store must report NotFound, not {:?}",
                e.kind
            );
            eprintln!("skipping: no OS trust store on this host");
        }
    }
}
