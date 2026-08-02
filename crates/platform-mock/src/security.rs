//! `Csprng` mock: deterministic, not cryptographically secure — trades
//! realism for reproducible tests, the same tradeoff every other mock in
//! this crate makes (RFC v2 §5.1).
//!
//! A fixed-zero or all-`0xFF` fill would let a caller that never actually
//! reads `buf` pass silently, so this generates real (if non-crypto)
//! varying bytes via a small xorshift64* stream, seeded identically every
//! run for reproducibility across test invocations.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;

use platform::error::Result;
use platform::error::{ErrorKind, OsCode, PlatformError};
use platform::security::{
    CredentialStore, CredentialStoreStatus, Csprng, Sandbox, SandboxStatus, TrustAnchors,
};

/// The mock backend's [`Csprng`] capability. Not thread-safe (`Cell`,
/// like [`crate::net::MockTcpStream`]'s `read_timeout`) — this crate's
/// test doubles have never needed cross-thread sharing.
pub struct MockCsprng {
    state: Cell<u64>,
}

impl Default for MockCsprng {
    fn default() -> Self {
        // Any nonzero seed works for xorshift64*; fixed for reproducibility.
        MockCsprng {
            state: Cell::new(0x9E37_79B9_7F4A_7C15),
        }
    }
}

impl MockCsprng {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_u64(&self) -> u64 {
        let mut x = self.state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

impl Csprng for MockCsprng {
    fn fill_random(&self, buf: &mut [u8]) -> Result<()> {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

/// The mock backend's [`CredentialStore`] capability (rustils#76): a
/// faithful in-memory fake, like [`MockCsprng`] and unlike
/// [`MockSandbox`] below — unlike kernel-level process confinement, a
/// get/set secret store genuinely *can* be faked without lying about a
/// security property, so this actually stores what's set and returns it
/// on `get`, rather than perpetually reporting `Unsupported`.
#[derive(Default)]
pub struct MockCredentialStore {
    store: RefCell<HashMap<(String, String), Vec<u8>>>,
}

impl MockCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for MockCredentialStore {
    fn available(&self) -> CredentialStoreStatus {
        CredentialStoreStatus::Available
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>> {
        let key = (service.to_string(), account.to_string());
        Ok(self.store.borrow().get(&key).cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<()> {
        let key = (service.to_string(), account.to_string());
        self.store.borrow_mut().insert(key, secret.to_vec());
        Ok(())
    }
}

/// The mock backend's [`Sandbox`] capability. There is no in-memory
/// equivalent of kernel-level process confinement to fake the way
/// `MockNet`/`MockDir` fake a socket or filesystem — a mock that claimed
/// `Enforced` here would be lying about a security property, worse than
/// not having a mock at all. Every call reports
/// [`SandboxStatus::Unsupported`], the same honest answer a real backend
/// with no confinement mechanism gives (see [`crate::MockCsprng`] for the
/// capability that *is* faithfully mockable).
pub struct MockSandbox;

impl Sandbox for MockSandbox {
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

/// The mock backend's [`TrustAnchors`] capability (rustils#88): a
/// faithful fake holding exactly the anchors the test put in it — like
/// [`MockCredentialStore`] and unlike [`MockSandbox`]. "Here is a list of
/// certificates" is genuinely fakeable without lying about a security
/// property, because this trait makes no claim about *validity*: it only
/// promises to hand back what the OS store holds, and a test store holds
/// what the test says it does.
///
/// This exists so a TLS-using consumer can build a hermetic trust
/// configuration — pin a throwaway test CA as the entire anchor set, with
/// no dependence on whatever the developer's machine happens to trust.
/// That is the whole reason the B1 slice landed here rather than in the
/// consumer (`docs/design-discussion-tls.md`).
///
/// [`Default`] is deliberately **not** an empty store: an empty one fails
/// closed (see [`TrustAnchors::load_anchors`]), so a test that forgot to
/// configure anchors would hit a confusing `NotFound` rather than the
/// working default every other mock here provides. `new()` seeds one
/// syntactically-plausible anchor; use [`MockTrustAnchors::empty`] when
/// the fail-closed path is what's under test.
pub struct MockTrustAnchors {
    anchors: RefCell<Vec<Vec<u8>>>,
}

impl Default for MockTrustAnchors {
    fn default() -> Self {
        // A DER SEQUENCE header over a short body: enough that a consumer
        // checking "did I get bytes shaped like a certificate" sees the
        // 0x30 tag real anchors start with, without pretending to be a
        // certificate any verifier would accept.
        Self {
            anchors: RefCell::new(vec![vec![0x30, 0x82, 0x01, 0x0a]]),
        }
    }
}

impl MockTrustAnchors {
    /// A store holding one placeholder anchor — the working default.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store holding exactly `anchors`.
    pub fn with_anchors(anchors: Vec<Vec<u8>>) -> Self {
        Self {
            anchors: RefCell::new(anchors),
        }
    }

    /// A store holding nothing, so [`TrustAnchors::load_anchors`] takes
    /// its fail-closed path. For testing a consumer's behavior on a
    /// machine with no readable trust store.
    pub fn empty() -> Self {
        Self::with_anchors(Vec::new())
    }

    /// Replace the stored anchor set.
    pub fn set_anchors(&self, anchors: Vec<Vec<u8>>) {
        *self.anchors.borrow_mut() = anchors;
    }
}

impl TrustAnchors for MockTrustAnchors {
    fn load_anchors(&self) -> Result<Vec<Vec<u8>>> {
        let anchors = self.anchors.borrow().clone();
        if anchors.is_empty() {
            // Same contract as every real backend: zero anchors is an
            // error, never `Ok(vec![])`.
            return Err(PlatformError::new(
                ErrorKind::NotFound,
                OsCode::None,
                "mock trust store is empty",
            ));
        }
        Ok(anchors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_store_round_trips_and_distinguishes_accounts() {
        let store = MockCredentialStore::new();
        assert_eq!(store.available(), CredentialStoreStatus::Available);
        assert_eq!(store.get("svc", "alice").unwrap(), None);

        store.set("svc", "alice", b"alice-secret").unwrap();
        store.set("svc", "bob", b"bob-secret").unwrap();
        assert_eq!(
            store.get("svc", "alice").unwrap(),
            Some(b"alice-secret".to_vec())
        );
        assert_eq!(
            store.get("svc", "bob").unwrap(),
            Some(b"bob-secret".to_vec())
        );

        // Overwrite: `set` replaces, doesn't accumulate.
        store.set("svc", "alice", b"new-secret").unwrap();
        assert_eq!(
            store.get("svc", "alice").unwrap(),
            Some(b"new-secret".to_vec())
        );
    }
}
