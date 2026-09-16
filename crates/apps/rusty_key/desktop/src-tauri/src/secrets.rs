//! OS-keychain secret storage (PRD 08): provider API keys live in the platform
//! keychain, never in JS memory. The frontend passes a key to `secrets_set` once;
//! Rust stores it and from then on reads it directly when building a provider.
//!
//! On Linux this resolves to the pure-Rust `linux-keyutils` backend (kernel
//! session keyring); macOS/Windows use their native keychains. Every operation
//! returns a [`BoundaryErrorPayload`] on failure so a missing backend surfaces as
//! a uniform `internal` rejection rather than a panic.

use crate::error::BoundaryErrorPayload;

/// The keychain service namespace for all Rusty Keys provider secrets.
pub const SERVICE: &str = "dev.rustykeys.desktop";

fn entry(provider: &str) -> Result<keyring::Entry, BoundaryErrorPayload> {
    keyring::Entry::new(SERVICE, provider)
        .map_err(|e| BoundaryErrorPayload::internal(format!("keychain open failed: {e}")))
}

/// Store `key` for `provider` in the OS keychain.
pub fn set(provider: &str, key: &str) -> Result<(), BoundaryErrorPayload> {
    entry(provider)?
        .set_password(key)
        .map_err(|e| BoundaryErrorPayload::internal(format!("keychain write failed: {e}")))
}

/// Retrieve `provider`'s key from the OS keychain (empty string when unset).
pub fn get(provider: &str) -> Result<String, BoundaryErrorPayload> {
    match entry(provider)?.get_password() {
        Ok(k) => Ok(k),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(e) => Err(BoundaryErrorPayload::internal(format!(
            "keychain read failed: {e}"
        ))),
    }
}

/// Delete `provider`'s key from the OS keychain (idempotent).
pub fn delete(provider: &str) -> Result<(), BoundaryErrorPayload> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(BoundaryErrorPayload::internal(format!(
            "keychain delete failed: {e}"
        ))),
    }
}
