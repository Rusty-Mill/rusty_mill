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

/// rustils#78's real Linux backend (Secret Service over D-Bus): with no
/// D-Bus session bus reachable, `available()` reports `Unavailable` — a
/// real mechanism exists on this OS, it's just not reachable right now —
/// and `get`/`set` surface that as a real `Err` rather than a silent
/// `Ok(None)`/`Ok(())`, per the trait's own contract (a clean miss and
/// "the store isn't reachable" are different claims). Live round-trip
/// coverage against a real, reachable Secret Service lives in
/// `tests/secret_service.rs`, which spawns its own `dbus-daemon`/
/// `gnome-keyring-daemon` pair and connects to it directly by address,
/// never touching `DBUS_SESSION_BUS_ADDRESS`.
///
/// `LinuxCredentialStore`'s public API has no way to inject a specific
/// bus address — like any ordinary D-Bus client, it discovers the
/// session bus the standard way, via this env var — so this test has to
/// clear it itself rather than simply relying on the ambient environment
/// having none: CI's own D-Bus session (started for *other* tests that
/// need a real reachable Secret Service, e.g. nexus-security's) would
/// otherwise make this negative case impossible to observe there.
///
/// # Safety / soundness
/// `env::remove_var`/`set_var` mutate genuinely process-global state, so
/// this is only sound because this workspace's CI runs tests through
/// `cargo nextest`, which gives every `#[test]` its own process (see
/// `.config/nextest.toml`'s own doc comment) — no other test, in this
/// file or the rest of this crate, can observe this mutation. Plain
/// `cargo test` shares one process across every test in this binary; the
/// other two tests in this file don't read this variable, so a
/// same-process race here couldn't corrupt an assertion, only formally
/// violate `env::set_var`'s documented single-threaded-mutation
/// requirement -- a real caveat for anyone running this file outside
/// nextest, not a functional risk under it.
///
/// Backend-specific by nature, so it stays here rather than moving into
/// the shared sets: "no bus reachable" is not a state Windows or macOS
/// has.
#[test]
#[allow(unsafe_code)]
fn linux_credential_store_reports_unavailable_with_no_bus_reachable() {
    let previous = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
    // SAFETY: see the doc comment above -- sound under this workspace's
    // actual CI test runner (nextest, one process per test), not
    // necessarily under a shared-process `cargo test` run.
    unsafe {
        std::env::remove_var("DBUS_SESSION_BUS_ADDRESS");
    }

    let store = platform_linux::LinuxCredentialStore;
    let result = std::panic::catch_unwind(|| {
        assert_eq!(store.available(), CredentialStoreStatus::Unavailable);
        assert!(store.get("svc", "acct").is_err());
        assert!(store.set("svc", "acct", b"secret").is_err());
    });

    // SAFETY: same as above; restores exactly what was observed before
    // this test touched it, regardless of whether the assertions passed.
    unsafe {
        match previous {
            Some(value) => std::env::set_var("DBUS_SESSION_BUS_ADDRESS", value),
            None => std::env::remove_var("DBUS_SESSION_BUS_ADDRESS"),
        }
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
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
