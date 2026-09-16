//! Index key management.
//!
//! The key is generated on this machine, stored in the OS keychain, and never
//! written into the file it unlocks. The critical behaviour here is the
//! distinction between *no key yet* and *keychain unreadable*: the first is
//! normal first-run and creates a key, the second is fatal. Collapsing them
//! would let a transient keychain error look like a fresh install and silently
//! rebuild an index that was never actually lost.

use crate::{Error, Result};
use rand::RngCore;

const SERVICE: &str = "site.myinventory.app";
const ACCOUNT: &str = "index-key";

/// Test/CI escape hatch: a hex key supplied directly. Never consulted when a
/// real keychain entry can be reached, and documented as unsuitable for real
/// use because it puts the key in the process environment.
pub const KEY_ENV: &str = "INVENTORY_INDEX_KEY";

pub trait KeyProvider: Send + Sync {
    /// Hex-encoded 256-bit key, created on first call and stable thereafter.
    fn get_or_create(&self) -> Result<String>;
    /// Is there already a stored key?
    ///
    /// Lets a caller tell "first run" from "the key that opened this index is
    /// gone" — two situations that otherwise look identical right up until the
    /// index fails to decrypt.
    fn exists(&self) -> Result<bool>;
    /// Remove the stored key. Only used by `inv reset`.
    fn forget(&self) -> Result<()>;
    fn describe(&self) -> String;
}

fn generate_key_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The real provider: macOS Keychain, Windows Credential Manager, or
/// Secret Service on Linux.
pub struct OsKeychain;

impl KeyProvider for OsKeychain {
    fn get_or_create(&self) -> Result<String> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| Error::KeyUnavailable(format!("could not address the keychain: {e}")))?;

        match entry.get_password() {
            Ok(k) if !k.trim().is_empty() => Ok(k),
            // First run, or the entry was deleted: mint one.
            Ok(_) | Err(keyring::Error::NoEntry) => {
                let key = generate_key_hex();
                entry.set_password(&key).map_err(|e| {
                    Error::KeyUnavailable(format!("could not store a new key: {e}"))
                })?;
                Ok(key)
            }
            // Anything else — locked keychain, denied access, broken daemon —
            // is fatal. We do not rebuild.
            Err(e) => Err(Error::KeyUnavailable(e.to_string())),
        }
    }

    fn exists(&self) -> Result<bool> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| Error::KeyUnavailable(format!("could not address the keychain: {e}")))?;
        match entry.get_password() {
            Ok(k) => Ok(!k.trim().is_empty()),
            Err(keyring::Error::NoEntry) => Ok(false),
            // An unreadable keychain is not the same as an absent key, and
            // must not be reported as one.
            Err(e) => Err(Error::KeyUnavailable(e.to_string())),
        }
    }

    fn forget(&self) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| Error::KeyUnavailable(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::KeyUnavailable(e.to_string())),
        }
    }

    fn describe(&self) -> String {
        if cfg!(target_os = "macos") {
            "macOS Keychain".into()
        } else if cfg!(target_os = "windows") {
            "Windows Credential Manager".into()
        } else {
            "Secret Service".into()
        }
    }
}

/// A key supplied directly rather than fetched from a keychain. Backs the
/// `INVENTORY_INDEX_KEY` escape hatch and is what tests use so the suite never
/// depends on a keychain daemon being present.
pub struct StaticKey {
    hex: String,
    origin: String,
}

impl StaticKey {
    pub fn new(hex: impl Into<String>) -> Self {
        StaticKey {
            hex: hex.into(),
            origin: "supplied key".into(),
        }
    }

    fn from_env(hex: String) -> Self {
        StaticKey {
            hex,
            origin: format!("{KEY_ENV} environment variable"),
        }
    }
}

impl KeyProvider for StaticKey {
    fn get_or_create(&self) -> Result<String> {
        Ok(self.hex.clone())
    }
    fn exists(&self) -> Result<bool> {
        Ok(!self.hex.trim().is_empty())
    }
    fn forget(&self) -> Result<()> {
        Ok(())
    }
    fn describe(&self) -> String {
        self.origin.clone()
    }
}

/// Pick a provider. Environment key wins when present so that headless runs
/// and tests never touch — or depend on — a real keychain daemon.
pub fn default_provider() -> Box<dyn KeyProvider> {
    match std::env::var(KEY_ENV) {
        Ok(k) if !k.trim().is_empty() => Box::new(StaticKey::from_env(k)),
        _ => Box::new(OsKeychain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_256_bit_hex_and_unique() {
        let a = generate_key_hex();
        let b = generate_key_hex();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn env_provider_is_selected_when_set() {
        std::env::set_var(KEY_ENV, "abc123");
        let p = default_provider();
        let got = p.get_or_create().unwrap();
        std::env::remove_var(KEY_ENV);
        assert_eq!(got, "abc123");
    }
}
